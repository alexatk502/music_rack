//! Low-pass gate (vactrol-style filter+VCA on a trigger) and a 4-pole Moog
//! ladder filter.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{ladder as lp, lpg};
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::{voct_to_hz, GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::{tanh_pade, undenorm, Smoothed};

// ---------------------------------------------------------------------------
// Low-pass gate
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum LpgResponse {
    LowPass,
    Vca,
    Both,
}

pub struct Lpg {
    svf: [Svf; MAX_CHANNELS],
    env: [f32; MAX_CHANNELS],
    trig_high: [bool; MAX_CHANNELS],
    freq: Smoothed,
    decay: f32,
    response: LpgResponse,
}

impl Lpg {
    pub fn new() -> Self {
        Self {
            svf: [Svf::default(); MAX_CHANNELS],
            env: [0.0; MAX_CHANNELS],
            trig_high: [false; MAX_CHANNELS],
            freq: Smoothed::new(2.0),
            decay: 0.2,
            response: LpgResponse::Both,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            lpg::FREQ => self.freq.set_target(value),
            lpg::DECAY => self.decay = value.clamp(0.005, 2.0),
            lpg::RESPONSE => {
                self.response = match value as u32 {
                    0 => LpgResponse::LowPass,
                    1 => LpgResponse::Vca,
                    _ => LpgResponse::Both,
                }
            }
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        trig: Option<&PortBuffer>,
        cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, trig, cv]);
        out.channels = channels;
        let freq = self.freq.current();
        for _ in 0..frames {
            self.freq.tick(ctx.smooth_k);
        }
        // Vactrol-like exponential decay coefficient.
        let dec_k = 1.0 - (-1.0 / (self.decay * ctx.sample_rate)).exp();
        let response = self.response;

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let trig_data = trig.map(|b| b.channel_or_broadcast(ch));
            let cv_data = cv.map(|b| b.channel_or_broadcast(ch));
            let svf = &mut self.svf[ch];
            for i in 0..frames {
                if let Some(t) = trig_data {
                    let high = t[i] >= GATE_THRESHOLD;
                    if high && !self.trig_high[ch] {
                        self.env[ch] = 1.0;
                    }
                    self.trig_high[ch] = high;
                }
                // Decay the ping envelope; CV holds the gate open as a floor.
                self.env[ch] -= dec_k * self.env[ch];
                let floor = cv_data.map_or(0.0, |c| (c[i] / GATE_HIGH).clamp(0.0, 1.0));
                let open = self.env[ch].max(floor);

                let x = in_data.map_or(0.0, |d| d[i]);
                // Opening raises the cutoff up to ~5 octaves above the closed
                // resting point.
                let cutoff = voct_to_hz(freq - 5.0 * (1.0 - open));
                let filtered = svf.tick(x, &SvfCoeffs::new(cutoff, 0.7071, ctx.sample_rate)).lp;
                out.data[ch][i] = match response {
                    LpgResponse::LowPass => filtered,
                    LpgResponse::Vca => x * open,
                    LpgResponse::Both => filtered * open,
                };
            }
            svf.undenorm();
        }
    }
}

impl Default for Lpg {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Moog ladder (4-pole, TPT one-poles with resonant feedback)
// ---------------------------------------------------------------------------

pub struct Ladder {
    z: [[f32; 4]; MAX_CHANNELS],
    last: [f32; MAX_CHANNELS],
    cutoff: Smoothed,
    res: Smoothed,
    drive: Smoothed,
}

impl Ladder {
    pub fn new() -> Self {
        Self {
            z: [[0.0; 4]; MAX_CHANNELS],
            last: [0.0; MAX_CHANNELS],
            cutoff: Smoothed::new(3.0),
            res: Smoothed::new(0.3),
            drive: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            lp::CUTOFF => self.cutoff.set_target(value),
            lp::RES => self.res.set_target(value.clamp(0.0, 1.0)),
            lp::DRIVE => self.drive.set_target(value.clamp(1.0, 8.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        cutoff_cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, cutoff_cv]);
        out.channels = channels;
        let mut cutoff = self.cutoff.current();
        let mut res = self.res.current();
        let mut drive = self.drive.current();
        for _ in 0..frames {
            cutoff = self.cutoff.tick(ctx.smooth_k);
            res = self.res.tick(ctx.smooth_k);
            drive = self.drive.tick(ctx.smooth_k);
        }
        // Resonance feedback amount (self-oscillates near 4).
        let fb = res * 4.0;

        for ch in 0..channels as usize {
            let cv = cutoff_cv.map(|b| b.channel_or_broadcast(ch));
            let z = &mut self.z[ch];
            for i in 0..frames {
                let fc = voct_to_hz(cutoff + cv.map_or(0.0, |c| c[i])).clamp(20.0, ctx.sample_rate * 0.45);
                let g = (core::f32::consts::PI * fc / ctx.sample_rate).tan();
                let big_g = g / (1.0 + g);

                let x = input.map_or(0.0, |b| b.channel_or_broadcast(ch)[i]);
                // Feedback uses the previous output sample (stable, slightly
                // detunes resonance — the classic "naive" Moog).
                let mut s = tanh_pade((x * drive / 5.0) - fb * self.last[ch]);
                for stage in z.iter_mut() {
                    let v = (s - *stage) * big_g;
                    let y = v + *stage;
                    *stage = undenorm(y + v);
                    s = y;
                }
                self.last[ch] = s;
                out.data[ch][i] = s * 5.0;
            }
        }
    }
}

impl Default for Ladder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;
    use rack_dsp::volts::AUDIO_PEAK;

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|&s| (s * s) as f64).sum::<f64>() / buf.len() as f64).sqrt() as f32
    }

    #[test]
    fn lpg_pings_on_trigger_then_closes() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lpg = Lpg::new();
        lpg.set_param(lpg::DECAY, 0.05);
        let mut out = PortBuffer::silent();
        // Constant tone into the input.
        let mut input = PortBuffer::silent();
        input.data[0] = [AUDIO_PEAK; BLOCK];

        // Closed (no trigger): output ~silent.
        let silent_trig = PortBuffer::silent();
        for _ in 0..50 {
            lpg.process(&ctx, Some(&input), Some(&silent_trig), None, &mut out, BLOCK);
        }
        assert!(rms(&out.data[0][..BLOCK]) < 0.2, "LPG leaked while closed");

        // Trigger: opens, passes signal.
        let mut trig = PortBuffer::silent();
        trig.data[0][0] = 10.0;
        lpg.process(&ctx, Some(&input), Some(&trig), None, &mut out, BLOCK);
        let open = rms(&out.data[0][..BLOCK]);
        assert!(open > 1.0, "LPG didn't open on trigger: {open}");

        // Decays back toward closed.
        let zero = PortBuffer::silent();
        for _ in 0..400 {
            lpg.process(&ctx, Some(&input), Some(&zero), None, &mut out, BLOCK);
        }
        assert!(rms(&out.data[0][..BLOCK]) < open * 0.1, "LPG didn't close");
    }

    #[test]
    fn ladder_lowpasses_and_stays_stable() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lad = Ladder::new();
        lad.cutoff.set_immediate(-1.0); // low cutoff
        lad.res.set_immediate(0.9); // high resonance

        let mut input = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        let mut phase = 0.0f32;
        // Bright content well above cutoff should be attenuated; output bounded.
        let mut in_e = 0.0f64;
        let mut out_e = 0.0f64;
        let mut peak = 0.0f32;
        for block in 0..1000 {
            for i in 0..BLOCK {
                input.data[0][i] = (2.0 * phase - 1.0) * AUDIO_PEAK; // saw @ 1kHz
                phase += 1000.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            lad.process(&ctx, Some(&input), None, &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
            if block > 200 {
                for i in 0..BLOCK {
                    in_e += (input.data[0][i] as f64).powi(2);
                    out_e += (out.data[0][i] as f64).powi(2);
                }
            }
        }
        assert!(peak < 30.0, "ladder blew up: {peak}");
        assert!((out_e / in_e).sqrt() < 0.6, "ladder didn't attenuate highs");
    }
}
