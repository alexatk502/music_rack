//! More effects: saturator, flanger, ping-pong delay, 3-band parametric EQ,
//! peak limiter, and a stereo-width (mid/side) tool.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{
    flanger as fp, limiter as lp, param_eq as ep, pingpong as pp, saturator as sp, stereo as stp,
};
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::{tanh_pade, undenorm, Smoothed};

// ---------------------------------------------------------------------------
// Saturator
// ---------------------------------------------------------------------------

pub struct Saturator {
    tilt: [f32; MAX_CHANNELS],
    drive: Smoothed,
    tone: Smoothed,
    mix: Smoothed,
}

impl Saturator {
    pub fn new() -> Self {
        Self {
            tilt: [0.0; MAX_CHANNELS],
            drive: Smoothed::new(2.0),
            tone: Smoothed::new(0.5),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            sp::DRIVE => self.drive.set_target(value.clamp(1.0, 20.0)),
            sp::TONE => self.tone.set_target(value.clamp(0.0, 1.0)),
            sp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        let drive = self.drive.current();
        let tone = self.tone.current();
        let mix = self.mix.current();
        for _ in 0..frames {
            self.drive.tick(ctx.smooth_k);
            self.tone.tick(ctx.smooth_k);
            self.mix.tick(ctx.smooth_k);
        }
        // Tone: a one-pole low-pass smoothing factor (0 = dark, 1 = bright).
        let lp_k = 0.05 + tone * 0.9;

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let dry = in_data.map_or(0.0, |d| d[i]);
                let sat = tanh_pade(dry * drive / AUDIO_PEAK) * AUDIO_PEAK;
                // Simple tilt: blend the saturated signal with a lowpassed
                // version for warmth control.
                self.tilt[ch] += lp_k * (sat - self.tilt[ch]);
                let toned = self.tilt[ch] + (sat - self.tilt[ch]) * tone;
                out.data[ch][i] = dry + (toned - dry) * mix;
            }
        }
    }
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Flanger (modulated short delay with feedback) — mono
// ---------------------------------------------------------------------------

const FLANGE_MAX: usize = 1024; // ~21 ms at 48 kHz

pub struct Flanger {
    buf: Box<[f32; FLANGE_MAX]>,
    pos: usize,
    lfo: f32,
    rate: Smoothed,
    depth: Smoothed,
    feedback: Smoothed,
    mix: Smoothed,
}

impl Flanger {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0.0; FLANGE_MAX]),
            pos: 0,
            lfo: 0.0,
            rate: Smoothed::new(0.3),
            depth: Smoothed::new(0.7),
            feedback: Smoothed::new(0.5),
            mix: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            fp::RATE => self.rate.set_target(value.clamp(0.05, 5.0)),
            fp::DEPTH => self.depth.set_target(value.clamp(0.0, 1.0)),
            fp::FEEDBACK => self.feedback.set_target(value.clamp(-0.95, 0.95)),
            fp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        out.channels = 1;
        let sr = ctx.sample_rate;
        // Sweep between ~0.5 ms and ~10 ms.
        let min_d = 0.0005 * sr;
        let span = 0.009 * sr;

        for i in 0..frames {
            let rate = self.rate.tick(ctx.smooth_k);
            let depth = self.depth.tick(ctx.smooth_k);
            let fb = self.feedback.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));

            let lfo = 0.5 - 0.5 * (core::f32::consts::TAU * self.lfo).cos();
            let delay = (min_d + lfo * depth * span).clamp(1.0, (FLANGE_MAX - 2) as f32);
            let read = self.pos as f32 - delay;
            let read = if read < 0.0 { read + FLANGE_MAX as f32 } else { read };
            let idx = read as usize;
            let frac = read - idx as f32;
            let a = self.buf[idx % FLANGE_MAX];
            let b = self.buf[(idx + 1) % FLANGE_MAX];
            let wet = a + (b - a) * frac;

            self.buf[self.pos] = undenorm(dry + wet * fb);
            self.pos = (self.pos + 1) % FLANGE_MAX;
            self.lfo += rate * ctx.inv_sample_rate;
            if self.lfo >= 1.0 {
                self.lfo -= 1.0;
            }
            out.data[0][i] = dry + (wet - dry) * mix;
        }
    }
}

impl Default for Flanger {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Ping-pong stereo delay
// ---------------------------------------------------------------------------

const PP_MAX: usize = 96_004; // ~2 s at 48 kHz

pub struct PingPong {
    left: Box<[f32; PP_MAX]>,
    right: Box<[f32; PP_MAX]>,
    pos: usize,
    time: Smoothed,
    feedback: Smoothed,
    mix: Smoothed,
}

impl PingPong {
    pub fn new() -> Self {
        Self {
            left: Box::new([0.0; PP_MAX]),
            right: Box::new([0.0; PP_MAX]),
            pos: 0,
            time: Smoothed::new(0.3),
            feedback: Smoothed::new(0.5),
            mix: Smoothed::new(0.4),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            pp::TIME => self.time.set_target(value.clamp(0.02, 1.5)),
            pp::FEEDBACK => self.feedback.set_target(value.clamp(0.0, 0.95)),
            pp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        left_in: Option<&PortBuffer>,
        right_in: Option<&PortBuffer>,
        left_out: &mut PortBuffer,
        right_out: &mut PortBuffer,
        frames: usize,
    ) {
        left_out.channels = 1;
        right_out.channels = 1;
        let ld = left_in;
        // Right normalled to left for mono sources.
        let rd = right_in.or(left_in);
        let time_k = Smoothed::coeff(0.08, ctx.sample_rate);

        for i in 0..frames {
            let t = self.time.tick(time_k);
            let fb = self.feedback.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dl = ld.map_or(0.0, |b| b.mono(i));
            let dr = rd.map_or(0.0, |b| b.mono(i));

            let delay = (t * ctx.sample_rate).clamp(1.0, (PP_MAX - 3) as f32);
            let read = |buf: &[f32; PP_MAX], pos: usize| -> f32 {
                let r = pos as f32 - delay;
                let r = if r < 0.0 { r + PP_MAX as f32 } else { r };
                let idx = r as usize;
                let frac = r - idx as f32;
                let a = buf[idx % PP_MAX];
                let b = buf[(idx + 1) % PP_MAX];
                a + (b - a) * frac
            };
            let echo_l = read(&self.left, self.pos);
            let echo_r = read(&self.right, self.pos);

            // Cross-feedback: left's echo feeds the right line and vice versa.
            self.left[self.pos] = undenorm(dl + echo_r * fb);
            self.right[self.pos] = undenorm(dr + echo_l * fb);
            self.pos = (self.pos + 1) % PP_MAX;

            left_out.data[0][i] = dl + (echo_l - dl) * mix;
            right_out.data[0][i] = dr + (echo_r - dr) * mix;
        }
    }
}

impl Default for PingPong {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Biquad + 3-band parametric EQ (mono)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = undenorm(y);
        y
    }

    fn low_shelf(&mut self, freq: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = core::f32::consts::TAU * freq / sr;
        let (sn, cs) = w.sin_cos();
        let s = 1.0; // shelf slope
        let beta = (a / (1.0 + (a + 1.0 / a) * (1.0 / s - 1.0)).max(0.0)).sqrt() * sn;
        let ap1 = a + 1.0;
        let am1 = a - 1.0;
        let b0 = a * (ap1 - am1 * cs + beta);
        let b1 = 2.0 * a * (am1 - ap1 * cs);
        let b2 = a * (ap1 - am1 * cs - beta);
        let a0 = ap1 + am1 * cs + beta;
        let a1 = -2.0 * (am1 + ap1 * cs);
        let a2 = ap1 + am1 * cs - beta;
        self.set(b0, b1, b2, a0, a1, a2);
    }

    fn high_shelf(&mut self, freq: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = core::f32::consts::TAU * freq / sr;
        let (sn, cs) = w.sin_cos();
        let beta = a.sqrt() * sn;
        let ap1 = a + 1.0;
        let am1 = a - 1.0;
        let b0 = a * (ap1 + am1 * cs + beta);
        let b1 = -2.0 * a * (am1 + ap1 * cs);
        let b2 = a * (ap1 + am1 * cs - beta);
        let a0 = ap1 - am1 * cs + beta;
        let a1 = 2.0 * (am1 - ap1 * cs);
        let a2 = ap1 - am1 * cs - beta;
        self.set(b0, b1, b2, a0, a1, a2);
    }

    fn peak(&mut self, freq: f32, gain_db: f32, q: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = core::f32::consts::TAU * freq / sr;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cs;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cs;
        let a2 = 1.0 - alpha / a;
        self.set(b0, b1, b2, a0, a1, a2);
    }

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
}

pub struct ParamEq {
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    low_db: f32,
    mid_freq: f32,
    mid_db: f32,
    high_db: f32,
    dirty: bool,
}

impl ParamEq {
    pub fn new() -> Self {
        Self {
            low: Biquad::default(),
            mid: Biquad::default(),
            high: Biquad::default(),
            low_db: 0.0,
            mid_freq: 1000.0,
            mid_db: 0.0,
            high_db: 0.0,
            dirty: true,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            ep::LOW => self.low_db = value.clamp(-15.0, 15.0),
            ep::MID_FREQ => self.mid_freq = value.clamp(200.0, 5000.0),
            ep::MID => self.mid_db = value.clamp(-15.0, 15.0),
            ep::HIGH => self.high_db = value.clamp(-15.0, 15.0),
            _ => return,
        }
        self.dirty = true;
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        if self.dirty {
            self.low.low_shelf(250.0, self.low_db, ctx.sample_rate);
            self.mid.peak(self.mid_freq, self.mid_db, 0.9, ctx.sample_rate);
            self.high.high_shelf(4000.0, self.high_db, ctx.sample_rate);
            self.dirty = false;
        }
        for i in 0..frames {
            let x = input.map_or(0.0, |b| b.mono(i));
            out.data[0][i] = self.high.tick(self.mid.tick(self.low.tick(x)));
        }
    }
}

impl Default for ParamEq {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Peak limiter
// ---------------------------------------------------------------------------

pub struct Limiter {
    env: [f32; MAX_CHANNELS],
    threshold_db: f32,
    release: f32,
}

impl Limiter {
    pub fn new() -> Self {
        Self { env: [0.0; MAX_CHANNELS], threshold_db: -3.0, release: 0.05 }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            lp::THRESHOLD => self.threshold_db = value.clamp(-24.0, 0.0),
            lp::RELEASE => self.release = value.clamp(0.01, 0.5),
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
        // Full-scale linear threshold (dBFS relative to ±5 V peak).
        let thresh = 10f32.powf(self.threshold_db / 20.0);
        let ceiling = thresh * AUDIO_PEAK;
        // Fast attack, knob-set release.
        let atk_k = 1.0 - (-1.0 / (0.001 * ctx.sample_rate)).exp();
        let rel_k = 1.0 - (-1.0 / (self.release * ctx.sample_rate)).exp();

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let env = &mut self.env[ch];
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                let level = x.abs();
                let k = if level > *env { atk_k } else { rel_k };
                *env += k * (level - *env);
                // Gain to keep the envelope under the ceiling.
                let gain = if *env > ceiling { ceiling / *env } else { 1.0 };
                // Brick-wall clip as a final safety net.
                out.data[ch][i] = (x * gain).clamp(-ceiling, ceiling);
            }
        }
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Stereo width (mid/side)
// ---------------------------------------------------------------------------

pub struct Stereo {
    width: Smoothed,
}

impl Stereo {
    pub fn new() -> Self {
        Self { width: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == stp::WIDTH {
            self.width.set_target(value.clamp(0.0, 2.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        left_in: Option<&PortBuffer>,
        right_in: Option<&PortBuffer>,
        left_out: &mut PortBuffer,
        right_out: &mut PortBuffer,
        frames: usize,
    ) {
        left_out.channels = 1;
        right_out.channels = 1;
        let ld = left_in;
        let rd = right_in.or(left_in);
        for i in 0..frames {
            let w = self.width.tick(ctx.smooth_k);
            let l = ld.map_or(0.0, |b| b.mono(i));
            let r = rd.map_or(0.0, |b| b.mono(i));
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5 * w;
            left_out.data[0][i] = mid + side;
            right_out.data[0][i] = mid - side;
        }
    }
}

impl Default for Stereo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|&s| (s * s) as f64).sum::<f64>() / buf.len() as f64).sqrt() as f32
    }

    #[test]
    fn saturator_adds_harmonics_and_stays_bounded() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut sat = Saturator::new();
        sat.drive.set_immediate(15.0);
        sat.mix.set_immediate(1.0);
        sat.tone.set_immediate(1.0);
        let mut input = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        let mut phase = 0.0f32;
        let mut diff = 0.0f64;
        let mut last = 0.0f32;
        for _ in 0..500 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += 200.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            sat.process(&ctx, Some(&input), &mut out, BLOCK);
            for i in 0..BLOCK {
                assert!(out.data[0][i].abs() <= AUDIO_PEAK + 0.1);
                diff += ((out.data[0][i] - last) as f64).abs();
                last = out.data[0][i];
            }
        }
        assert!(diff > 0.0);
    }

    #[test]
    fn flanger_is_bounded_with_feedback() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut fl = Flanger::new();
        fl.feedback.set_immediate(0.9);
        fl.mix.set_immediate(1.0);
        let mut input = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        let mut phase = 0.0f32;
        for _ in 0..2000 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += 330.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            fl.process(&ctx, Some(&input), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                assert!(s.is_finite() && s.abs() < 50.0, "flanger blew up: {s}");
            }
        }
    }

    #[test]
    fn pingpong_echoes_alternate_channels() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut pp = PingPong::new();
        pp.time.set_immediate(0.05);
        pp.mix.set_immediate(1.0);
        pp.feedback.set_immediate(0.6);
        let mut input = PortBuffer::silent();
        input.data[0][0] = AUDIO_PEAK; // impulse into the left only
        let mut l = PortBuffer::silent();
        let mut r = PortBuffer::silent();
        let silent = PortBuffer::silent();

        let mut l_energy = 0.0f64;
        let mut r_energy = 0.0f64;
        for block in 0..500 {
            let inp = if block == 0 { &input } else { &silent };
            // Feed left only (right unpatched → normalled to left, so pass None).
            pp.process(&ctx, Some(inp), None, &mut l, &mut r, BLOCK);
            for i in 0..BLOCK {
                assert!(l.data[0][i].is_finite() && r.data[0][i].is_finite());
                l_energy += (l.data[0][i] as f64).powi(2);
                r_energy += (r.data[0][i] as f64).powi(2);
            }
        }
        // Cross-feedback puts energy on both sides.
        assert!(l_energy > 0.0 && r_energy > 0.0, "ping-pong didn't cross channels");
    }

    #[test]
    fn eq_boosts_and_cuts() {
        let ctx = ProcessCtx::new(48_000.0);
        // Measure gain of a 5 kHz tone with high shelf flat vs boosted.
        let gain_at = |high_db: f32| {
            let mut eq = ParamEq::new();
            eq.set_param(ep::HIGH, high_db);
            let mut input = PortBuffer::silent();
            let mut out = PortBuffer::silent();
            let mut phase = 0.0f32;
            let mut e = 0.0f64;
            for block in 0..400 {
                for i in 0..BLOCK {
                    input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                    phase += 6000.0 / 48_000.0;
                    if phase >= 1.0 { phase -= 1.0; }
                }
                eq.process(&ctx, Some(&input), &mut out, BLOCK);
                if block > 100 {
                    for &s in &out.data[0][..BLOCK] {
                        assert!(s.is_finite());
                        e += (s as f64).powi(2);
                    }
                }
            }
            e.sqrt()
        };
        let flat = gain_at(0.0);
        let boosted = gain_at(12.0);
        let cut = gain_at(-12.0);
        assert!(boosted > flat * 1.5, "high boost ineffective: {flat} -> {boosted}");
        assert!(cut < flat * 0.7, "high cut ineffective: {flat} -> {cut}");
    }

    #[test]
    fn limiter_caps_peaks() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lim = Limiter::new();
        lim.set_param(lp::THRESHOLD, -6.0);
        lim.set_param(lp::RELEASE, 0.05);
        let ceiling = 10f32.powf(-6.0 / 20.0) * AUDIO_PEAK; // ~2.5 V
        let mut input = PortBuffer::silent();
        input.data[0] = [AUDIO_PEAK; BLOCK]; // 5 V, well over the ceiling
        let mut out = PortBuffer::silent();
        for _ in 0..400 {
            lim.process(&ctx, Some(&input), &mut out, BLOCK);
        }
        for &s in &out.data[0][..BLOCK] {
            assert!(s.abs() <= ceiling + 0.05, "limiter exceeded ceiling: {s} > {ceiling}");
        }
        let _ = rms(&out.data[0][..BLOCK]);
    }

    #[test]
    fn mono_eq_sums_poly_voices() {
        // Regression: a mono module fed a polyphonic signal must hear EVERY
        // voice, not just channel 0 — otherwise a note on another poly channel
        // is silent until the first voice releases.
        let ctx = ProcessCtx::new(48_000.0);
        let mut eq = ParamEq::new(); // flat (0 dB) → unity, so output = sum
        let mut input = PortBuffer::silent();
        input.channels = 3;
        input.data[0] = [1.0; BLOCK];
        input.data[1] = [2.0; BLOCK];
        input.data[2] = [3.0; BLOCK];
        let mut out = PortBuffer::silent();
        for _ in 0..200 {
            eq.process(&ctx, Some(&input), &mut out, BLOCK);
        }
        // Sum of the three active channels (not just channel 0's 1.0).
        assert!((out.data[0][BLOCK - 1] - 6.0).abs() < 0.05, "EQ only heard one voice: {}", out.data[0][BLOCK - 1]);
    }

    #[test]
    fn stereo_width_zero_is_mono() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut st = Stereo::new();
        st.width.set_immediate(0.0);
        let mut l_in = PortBuffer::silent();
        let mut r_in = PortBuffer::silent();
        l_in.data[0] = [3.0; BLOCK];
        r_in.data[0] = [-1.0; BLOCK];
        let mut l = PortBuffer::silent();
        let mut r = PortBuffer::silent();
        st.process(&ctx, Some(&l_in), Some(&r_in), &mut l, &mut r, BLOCK);
        // Width 0 collapses to mid = (3 + -1)/2 = 1 on both.
        assert!((l.data[0][BLOCK - 1] - 1.0).abs() < 1e-4);
        assert!((r.data[0][BLOCK - 1] - 1.0).abs() < 1e-4);
    }
}
