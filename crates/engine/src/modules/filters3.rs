//! Extra filters: a multi-output state-variable filter, an envelope-following
//! auto-wah, and a stereo dual filter with L/R cutoff spread.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::Smoothed;

// ---------------------------------------------------------------------------
// SVF: simultaneous low-pass / band-pass / high-pass / notch outputs.
// params: 0 cutoff (Hz), 1 res, 2 cv amount
// ---------------------------------------------------------------------------

pub struct SvfMulti {
    svf: [Svf; MAX_CHANNELS],
    cutoff: Smoothed,
    res: Smoothed,
    cv_amt: Smoothed,
}

impl SvfMulti {
    pub fn new() -> Self {
        Self {
            svf: [Svf::default(); MAX_CHANNELS],
            cutoff: Smoothed::new(1000.0),
            res: Smoothed::new(0.7),
            cv_amt: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.cutoff.set_target(value.clamp(20.0, 16000.0)),
            1 => self.res.set_target(value.clamp(0.5, 20.0)),
            2 => self.cv_amt.set_target(value.clamp(-1.0, 1.0)),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        cv: Option<&PortBuffer>,
        lp: &mut PortBuffer,
        bp: &mut PortBuffer,
        hp: &mut PortBuffer,
        notch: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, cv]);
        lp.channels = channels;
        bp.channels = channels;
        hp.channels = channels;
        notch.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let cv_data = cv.map(|b| b.channel_or_broadcast(ch));
            self.svf[ch].undenorm();
            for i in 0..frames {
                let cutoff = self.cutoff.tick(ctx.smooth_k);
                let res = self.res.tick(ctx.smooth_k);
                let cv_amt = self.cv_amt.tick(ctx.smooth_k);
                // 1 V/oct-ish CV: each volt of CV (×amount) shifts an octave.
                let modv = cv_data.map_or(0.0, |c| c[i]) * cv_amt;
                let f = (cutoff * 2.0f32.powf(modv)).clamp(20.0, ctx.sample_rate * 0.45);
                let coeffs = SvfCoeffs::new(f, res, ctx.sample_rate);
                let o = self.svf[ch].tick(in_data.map_or(0.0, |d| d[i]), &coeffs);
                lp.data[ch][i] = o.lp;
                bp.data[ch][i] = o.bp;
                hp.data[ch][i] = o.hp;
                notch.data[ch][i] = o.lp + o.hp;
            }
        }
    }
}

impl Default for SvfMulti {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Auto-Wah: an envelope follower sweeps a resonant band-pass.
// params: 0 sensitivity, 1 range (octaves), 2 res, 3 base (Hz)
// ---------------------------------------------------------------------------

pub struct AutoWah {
    svf: [Svf; MAX_CHANNELS],
    env: [f32; MAX_CHANNELS],
    sens: Smoothed,
    range: Smoothed,
    res: Smoothed,
    base: Smoothed,
}

impl AutoWah {
    pub fn new() -> Self {
        Self {
            svf: [Svf::default(); MAX_CHANNELS],
            env: [0.0; MAX_CHANNELS],
            sens: Smoothed::new(0.5),
            range: Smoothed::new(0.6),
            res: Smoothed::new(4.0),
            base: Smoothed::new(300.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.sens.set_target(value.clamp(0.0, 1.0)),
            1 => self.range.set_target(value.clamp(0.0, 1.0)),
            2 => self.res.set_target(value.clamp(0.5, 15.0)),
            3 => self.base.set_target(value.clamp(20.0, 2000.0)),
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
        // Envelope follower coefficients (~5 ms attack, ~80 ms release).
        let atk = 1.0 - (-1.0 / (0.005 * ctx.sample_rate)).exp();
        let rel = 1.0 - (-1.0 / (0.080 * ctx.sample_rate)).exp();
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            self.svf[ch].undenorm();
            for i in 0..frames {
                let sens = self.sens.tick(ctx.smooth_k);
                let range = self.range.tick(ctx.smooth_k);
                let res = self.res.tick(ctx.smooth_k);
                let base = self.base.tick(ctx.smooth_k);
                let x = in_data.map_or(0.0, |d| d[i]);
                let rect = (x / AUDIO_PEAK).abs();
                let k = if rect > self.env[ch] { atk } else { rel };
                self.env[ch] += k * (rect - self.env[ch]);
                // Sweep up to `range`×6 octaves above base with the envelope.
                let oct = self.env[ch] * sens * range * 6.0;
                let f = (base * 2.0f32.powf(oct)).clamp(20.0, ctx.sample_rate * 0.45);
                let coeffs = SvfCoeffs::new(f, res, ctx.sample_rate);
                out.data[ch][i] = self.svf[ch].tick(x, &coeffs).bp;
            }
        }
    }
}

impl Default for AutoWah {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Dual Filter: a stereo filter with independent L/R cutoff spread.
// params: 0 cutoff (Hz), 1 res, 2 mode (LP/BP/HP), 3 spread
// ---------------------------------------------------------------------------

pub struct DualFilter {
    left: [Svf; MAX_CHANNELS],
    right: [Svf; MAX_CHANNELS],
    cutoff: Smoothed,
    res: Smoothed,
    spread: Smoothed,
    mode: u32,
}

impl DualFilter {
    pub fn new() -> Self {
        Self {
            left: [Svf::default(); MAX_CHANNELS],
            right: [Svf::default(); MAX_CHANNELS],
            cutoff: Smoothed::new(1000.0),
            res: Smoothed::new(1.0),
            spread: Smoothed::new(0.0),
            mode: 0,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.cutoff.set_target(value.clamp(20.0, 16000.0)),
            1 => self.res.set_target(value.clamp(0.5, 15.0)),
            2 => self.mode = (value as u32).min(2),
            3 => self.spread.set_target(value.clamp(-1.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn pick(mode: u32, o: &rack_dsp::svf::SvfOut) -> f32 {
        match mode {
            1 => o.bp,
            2 => o.hp,
            _ => o.lp,
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        in_l: Option<&PortBuffer>,
        in_r: Option<&PortBuffer>,
        out_l: &mut PortBuffer,
        out_r: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[in_l, in_r]);
        out_l.channels = channels;
        out_r.channels = channels;
        let mode = self.mode;
        for ch in 0..channels as usize {
            let l = in_l.map(|b| b.channel_or_broadcast(ch));
            // Right normals to left when unpatched (VCV convention).
            let r = in_r.or(in_l).map(|b| b.channel_or_broadcast(ch));
            self.left[ch].undenorm();
            self.right[ch].undenorm();
            for i in 0..frames {
                let cutoff = self.cutoff.tick(ctx.smooth_k);
                let res = self.res.tick(ctx.smooth_k);
                let spread = self.spread.tick(ctx.smooth_k);
                let fl = (cutoff * 2.0f32.powf(spread * 0.5)).clamp(20.0, ctx.sample_rate * 0.45);
                let fr = (cutoff * 2.0f32.powf(-spread * 0.5)).clamp(20.0, ctx.sample_rate * 0.45);
                let cl = SvfCoeffs::new(fl, res, ctx.sample_rate);
                let cr = SvfCoeffs::new(fr, res, ctx.sample_rate);
                let ol = self.left[ch].tick(l.map_or(0.0, |d| d[i]), &cl);
                let or = self.right[ch].tick(r.map_or(0.0, |d| d[i]), &cr);
                out_l.data[ch][i] = Self::pick(mode, &ol);
                out_r.data[ch][i] = Self::pick(mode, &or);
            }
        }
    }
}

impl Default for DualFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn noise_buf(seed: &mut u32) -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        for s in b.data[0].iter_mut() {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            *s = (*seed as f32 / u32::MAX as f32 - 0.5) * 2.0 * AUDIO_PEAK;
        }
        b
    }

    #[test]
    fn new_filters_stay_finite_at_high_resonance() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut seed = 1;
        let mut svf = SvfMulti::new();
        let mut wah = AutoWah::new();
        let mut dual = DualFilter::new();
        svf.set_param(1, 20.0); // max res
        wah.set_param(2, 15.0);
        dual.set_param(1, 15.0);
        dual.set_param(3, 1.0); // full spread
        for _ in 0..200 {
            let inp = noise_buf(&mut seed);
            let (mut lp, mut bp, mut hp, mut notch) = (
                PortBuffer::silent(),
                PortBuffer::silent(),
                PortBuffer::silent(),
                PortBuffer::silent(),
            );
            svf.process(&ctx, Some(&inp), None, &mut lp, &mut bp, &mut hp, &mut notch, BLOCK);
            let mut out = PortBuffer::silent();
            wah.process(&ctx, Some(&inp), &mut out, BLOCK);
            let (mut l, mut r) = (PortBuffer::silent(), PortBuffer::silent());
            dual.process(&ctx, Some(&inp), Some(&inp), &mut l, &mut r, BLOCK);
            for buf in [&lp, &bp, &hp, &notch, &out, &l, &r] {
                for &s in buf.data[0][..BLOCK].iter() {
                    assert!(s.is_finite() && s.abs() < 100.0, "filter blew up: {s}");
                }
            }
        }
    }
}
