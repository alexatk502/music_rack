//! Function generators: a Maths-style rise/fall function (one-shot or
//! cycling), a complex LFO with four phase-tapped outputs, and an envelope
//! follower.

use crate::buffer::{propagate_channels, PortBuffer};
use crate::ProcessCtx;
use rack_core::modules::params::{complexlfo as cp, envfollow as ep, maths as mp};
use rack_dsp::volts::{AUDIO_PEAK, GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::Smoothed;

// ---------------------------------------------------------------------------
// Maths-style function generator (mono)
// ---------------------------------------------------------------------------

pub struct Maths {
    level: f32,
    rising: bool,
    rise: f32,
    fall: f32,
    cycle: bool,
    trig_high: bool,
    eoc_timer: u32,
}

impl Maths {
    pub fn new() -> Self {
        Self {
            level: 0.0,
            rising: false,
            rise: 0.2,
            fall: 0.4,
            cycle: false,
            trig_high: false,
            eoc_timer: 0,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            mp::RISE => self.rise = value.clamp(0.001, 4.0),
            mp::FALL => self.fall = value.clamp(0.001, 4.0),
            mp::CYCLE => self.cycle = value >= 0.5,
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        trig: Option<&PortBuffer>,
        out: &mut PortBuffer,
        eoc: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        eoc.channels = 1;
        let trig_data = trig.map(|b| b.channel_or_broadcast(0));
        // Linear segment increments per sample.
        let rise_inc = 1.0 / (self.rise * ctx.sample_rate);
        let fall_inc = 1.0 / (self.fall * ctx.sample_rate);

        for i in 0..frames {
            if let Some(t) = trig_data {
                let high = t[i] >= GATE_THRESHOLD;
                if high && !self.trig_high {
                    self.rising = true; // (re)start the rise segment
                }
                self.trig_high = high;
            }

            if self.rising {
                self.level += rise_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.rising = false;
                }
            } else {
                self.level -= fall_inc;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    // End of cycle: emit a short trigger; restart if cycling.
                    self.eoc_timer = 48;
                    if self.cycle {
                        self.rising = true;
                    }
                }
            }

            out.data[0][i] = self.level * GATE_HIGH;
            eoc.data[0][i] = if self.eoc_timer > 0 { GATE_HIGH } else { 0.0 };
            self.eoc_timer = self.eoc_timer.saturating_sub(1);
        }
    }
}

impl Default for Maths {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Complex LFO: four outputs at 0°, 90°, 180°, 270°
// ---------------------------------------------------------------------------

pub struct ComplexLfo {
    phase: f32,
    rate: Smoothed,
}

impl ComplexLfo {
    pub fn new() -> Self {
        Self { phase: 0.0, rate: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == cp::RATE {
            self.rate.set_target(value.clamp(0.02, 20.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        rate_cv: Option<&PortBuffer>,
        outs: [&mut PortBuffer; 4],
        frames: usize,
    ) {
        let [o0, o90, o180, o270] = outs;
        for o in [&mut *o0, &mut *o90, &mut *o180, &mut *o270] {
            o.channels = 1;
        }
        let cv = rate_cv.map(|b| b.channel_or_broadcast(0));
        for i in 0..frames {
            let rate = (self.rate.tick(ctx.smooth_k) * cv.map_or(1.0, |c| 2f32.powf(c[i]))).clamp(0.01, 40.0);
            let p = self.phase;
            o0.data[0][i] = (core::f32::consts::TAU * p).sin() * AUDIO_PEAK;
            o90.data[0][i] = (core::f32::consts::TAU * (p + 0.25)).sin() * AUDIO_PEAK;
            o180.data[0][i] = (core::f32::consts::TAU * (p + 0.5)).sin() * AUDIO_PEAK;
            o270.data[0][i] = (core::f32::consts::TAU * (p + 0.75)).sin() * AUDIO_PEAK;
            self.phase += rate * ctx.inv_sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

impl Default for ComplexLfo {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Envelope follower
// ---------------------------------------------------------------------------

pub struct EnvFollower {
    env: [f32; rack_core::caps::MAX_CHANNELS],
    attack: Smoothed,
    release: Smoothed,
}

impl EnvFollower {
    pub fn new() -> Self {
        Self {
            env: [0.0; rack_core::caps::MAX_CHANNELS],
            attack: Smoothed::new(0.01),
            release: Smoothed::new(0.1),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            ep::ATTACK => self.attack.set_target(value.clamp(0.001, 0.5)),
            ep::RELEASE => self.release.set_target(value.clamp(0.001, 2.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        let atk = self.attack.current();
        let rel = self.release.current();
        for _ in 0..frames {
            self.attack.tick(ctx.smooth_k);
            self.release.tick(ctx.smooth_k);
        }
        let atk_k = 1.0 - (-1.0 / (atk * ctx.sample_rate)).exp();
        let rel_k = 1.0 - (-1.0 / (rel * ctx.sample_rate)).exp();

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let env = &mut self.env[ch];
            for i in 0..frames {
                let rect = in_data.map_or(0.0, |d| d[i]).abs();
                let k = if rect > *env { atk_k } else { rel_k };
                *env += k * (rect - *env);
                // Output as 0–10 V CV (input ±5 V → 0–10 V env).
                out.data[ch][i] = (*env / AUDIO_PEAK * GATE_HIGH).min(GATE_HIGH);
            }
        }
    }
}

impl Default for EnvFollower {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn maths_one_shot_rises_then_falls_to_zero() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut m = Maths::new();
        m.set_param(mp::RISE, 0.01);
        m.set_param(mp::FALL, 0.02);
        let mut out = PortBuffer::silent();
        let mut eoc = PortBuffer::silent();

        let mut trig = PortBuffer::silent();
        trig.data[0][0] = 10.0;
        m.process(&ctx, Some(&trig), &mut out, &mut eoc, BLOCK);

        // Run a while with no trigger; capture peak and final.
        let zero = PortBuffer::silent();
        let mut peak = 0.0f32;
        let mut eoc_fired = false;
        for _ in 0..200 {
            m.process(&ctx, Some(&zero), &mut out, &mut eoc, BLOCK);
            for i in 0..BLOCK {
                peak = peak.max(out.data[0][i]);
                if eoc.data[0][i] > 1.0 {
                    eoc_fired = true;
                }
            }
        }
        assert!(peak > 9.0, "rise didn't reach top: {peak}");
        assert!(eoc_fired, "no end-of-cycle trigger");
        assert!(out.data[0][BLOCK - 1] < 0.01, "didn't return to zero");
    }

    #[test]
    fn maths_cycle_oscillates() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut m = Maths::new();
        m.set_param(mp::RISE, 0.005);
        m.set_param(mp::FALL, 0.005);
        m.set_param(mp::CYCLE, 1.0);
        // Kick it off.
        let mut trig = PortBuffer::silent();
        trig.data[0][0] = 10.0;
        let mut out = PortBuffer::silent();
        let mut eoc = PortBuffer::silent();
        m.process(&ctx, Some(&trig), &mut out, &mut eoc, BLOCK);
        let zero = PortBuffer::silent();
        let mut rises = 0;
        let mut last = 0.0f32;
        let mut going_up_prev = true;
        for _ in 0..500 {
            m.process(&ctx, Some(&zero), &mut out, &mut eoc, BLOCK);
            for i in 0..BLOCK {
                let up = out.data[0][i] > last;
                if up && !going_up_prev {
                    rises += 1;
                }
                going_up_prev = up;
                last = out.data[0][i];
            }
        }
        assert!(rises > 3, "cycle not oscillating: {rises} troughs");
    }

    #[test]
    fn complex_lfo_outputs_are_phase_offset() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lfo = ComplexLfo::new();
        lfo.rate.set_immediate(2.0);
        let mut o = [PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent()];
        let [a, b, c, d] = &mut o;
        lfo.process(&ctx, None, [a, b, c, d], BLOCK);
        // At phase ~0: 0° ≈ 0 rising, 90° ≈ +peak, 180° ≈ 0 falling, 270° ≈ -peak.
        assert!(o[1].data[0][0] > o[0].data[0][0], "90° should lead 0°");
        assert!(o[3].data[0][0] < o[0].data[0][0], "270° should trail 0°");
    }

    #[test]
    fn envelope_follower_tracks_amplitude() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ef = EnvFollower::new();
        ef.set_param(ep::ATTACK, 0.005);
        ef.set_param(ep::RELEASE, 0.05);
        let mut out = PortBuffer::silent();

        // Loud sine → env rises toward ~10 V.
        let mut input = PortBuffer::silent();
        let mut phase = 0.0f32;
        for _ in 0..200 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += 300.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            ef.process(&ctx, Some(&input), &mut out, BLOCK);
        }
        let loud = out.data[0][BLOCK - 1];
        assert!(loud > 5.0, "env didn't rise on loud input: {loud}");

        // Silence → env falls back down.
        let silent = PortBuffer::silent();
        for _ in 0..400 {
            ef.process(&ctx, Some(&silent), &mut out, BLOCK);
        }
        assert!(out.data[0][BLOCK - 1] < 0.5, "env didn't release");
    }
}
