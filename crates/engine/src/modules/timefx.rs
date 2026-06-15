//! Time- and pitch-domain effects: tremolo, vibrato, tape delay, a granular
//! pitch shifter, a Bode-style frequency shifter, a shimmer reverb, and a
//! channel vocoder. The buffer-heavy ones run mono (poly inputs summed) to
//! keep memory bounded, matching the core delay/reverb convention.

use crate::buffer::{propagate_channels, PortBuffer};
use crate::ProcessCtx;
use core::f32::consts::TAU;
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::{tanh_pade, undenorm, Smoothed};

// ---------------------------------------------------------------------------
// Tremolo: amplitude modulation (poly).
// params: 0 rate, 1 depth, 2 shape (sine/square)
// ---------------------------------------------------------------------------

pub struct Tremolo {
    phase: f32,
    rate: Smoothed,
    depth: Smoothed,
    square: bool,
}

impl Tremolo {
    pub fn new() -> Self {
        Self { phase: 0.0, rate: Smoothed::new(4.0), depth: Smoothed::new(0.7), square: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.rate.set_target(value.clamp(0.05, 20.0)),
            1 => self.depth.set_target(value.clamp(0.0, 1.0)),
            2 => self.square = value as u32 == 1,
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
        let mut gain = [1.0f32; crate::buffer::BLOCK];
        for g in gain.iter_mut().take(frames) {
            let rate = self.rate.tick(ctx.smooth_k);
            let depth = self.depth.tick(ctx.smooth_k);
            self.phase = (self.phase + rate * ctx.inv_sample_rate).fract();
            let lfo = if self.square {
                if self.phase < 0.5 {
                    1.0
                } else {
                    0.0
                }
            } else {
                0.5 - 0.5 * (TAU * self.phase).cos()
            };
            *g = 1.0 - depth + depth * lfo;
        }
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                out.data[ch][i] = in_data.map_or(0.0, |d| d[i]) * gain[i];
            }
        }
    }
}

impl Default for Tremolo {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Vibrato: pitch wobble via a short modulated delay (poly).
// params: 0 rate, 1 depth
// ---------------------------------------------------------------------------

const VIB_LEN: usize = 2048; // ~42 ms at 48 kHz

pub struct Vibrato {
    buf: Box<[[f32; VIB_LEN]; crate::buffer::MAX_CHANNELS]>,
    write: usize,
    phase: f32,
    rate: Smoothed,
    depth: Smoothed,
}

impl Vibrato {
    pub fn new() -> Self {
        Self {
            buf: Box::new([[0.0; VIB_LEN]; crate::buffer::MAX_CHANNELS]),
            write: 0,
            phase: 0.0,
            rate: Smoothed::new(5.0),
            depth: Smoothed::new(0.3),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.rate.set_target(value.clamp(0.1, 12.0)),
            1 => self.depth.set_target(value.clamp(0.0, 1.0)),
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
        let base = 0.003 * ctx.sample_rate; // 3 ms nominal delay
        for i in 0..frames {
            let rate = self.rate.tick(ctx.smooth_k);
            let depth = self.depth.tick(ctx.smooth_k);
            self.phase = (self.phase + rate * ctx.inv_sample_rate).fract();
            let lfo = (TAU * self.phase).sin();
            let delay = base + lfo * depth * 0.004 * ctx.sample_rate;
            let w = self.write;
            for ch in 0..channels as usize {
                let x = input.map(|b| b.channel_or_broadcast(ch)).map_or(0.0, |d| d[i]);
                self.buf[ch][w] = x;
                let read = (w as f32 - delay + VIB_LEN as f32) % VIB_LEN as f32;
                let idx = read as usize;
                let frac = read - idx as f32;
                let a = self.buf[ch][idx % VIB_LEN];
                let b = self.buf[ch][(idx + 1) % VIB_LEN];
                out.data[ch][i] = a + (b - a) * frac;
            }
            self.write = (w + 1) % VIB_LEN;
        }
    }
}

impl Default for Vibrato {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tape Delay: wow/flutter modulated delay with filtered, saturated feedback.
// params: 0 time, 1 feedback, 2 tone, 3 wow, 4 mix
// ---------------------------------------------------------------------------

const TAPE_LEN: usize = 48_000 * 3 / 2 + 4; // 1.5 s at 48 kHz

pub struct TapeDelay {
    buf: Box<[f32; TAPE_LEN]>,
    write: usize,
    lp: f32,
    wow_phase: f32,
    time: Smoothed,
    feedback: Smoothed,
    tone: Smoothed,
    wow: Smoothed,
    mix: Smoothed,
}

impl TapeDelay {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0.0; TAPE_LEN]),
            write: 0,
            lp: 0.0,
            wow_phase: 0.0,
            time: Smoothed::new(0.3),
            feedback: Smoothed::new(0.4),
            tone: Smoothed::new(0.5),
            wow: Smoothed::new(0.2),
            mix: Smoothed::new(0.4),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.time.set_target(value.clamp(0.02, 1.5)),
            1 => self.feedback.set_target(value.clamp(0.0, 0.95)),
            2 => self.tone.set_target(value.clamp(0.0, 1.0)),
            3 => self.wow.set_target(value.clamp(0.0, 1.0)),
            4 => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        let time_k = Smoothed::coeff(0.080, ctx.sample_rate);
        let len = TAPE_LEN as f32;
        for i in 0..frames {
            let time = self.time.tick(time_k);
            let feedback = self.feedback.tick(ctx.smooth_k);
            let tone = self.tone.tick(ctx.smooth_k);
            let wow = self.wow.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);

            // Wow/flutter: ~0.7 Hz delay-time wobble.
            self.wow_phase = (self.wow_phase + 0.7 * ctx.inv_sample_rate).fract();
            let warble = (TAU * self.wow_phase).sin() * wow * 0.003 * ctx.sample_rate;
            let delay_samples = (time * ctx.sample_rate + warble).clamp(1.0, len - 3.0);
            let read_pos = self.write as f32 - delay_samples;
            let read_pos = if read_pos < 0.0 { read_pos + len } else { read_pos };
            let idx = read_pos as usize;
            let frac = read_pos - idx as f32;
            let a = self.buf[idx % TAPE_LEN];
            let b = self.buf[(idx + 1) % TAPE_LEN];
            let delayed = a + (b - a) * frac;

            let dry = input.map_or(0.0, |bb| bb.mono(i));
            // Feedback tone: one-pole low-pass darkens repeats.
            let lp_k = 0.1 + tone * 0.85;
            self.lp += lp_k * (delayed - self.lp);
            let fed = tanh_pade((dry + self.lp * feedback) / 10.0) * 10.0;
            self.buf[self.write] = undenorm(fed);
            self.write = (self.write + 1) % TAPE_LEN;
            out.data[0][i] = dry + (delayed - dry) * mix;
        }
    }
}

impl Default for TapeDelay {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Granular pitch shifter (mono): two crossfaded read taps drift through a
// delay window at the pitch ratio.
// params: 0 semitones, 1 fine (cents), 2 mix
// ---------------------------------------------------------------------------

const GRAIN_LEN: usize = 4096;
const GRAIN_WIN: f32 = 2048.0;

struct Grain {
    buf: Box<[f32; GRAIN_LEN]>,
    write: usize,
    ph: f32,
}

impl Grain {
    fn new() -> Self {
        Self { buf: Box::new([0.0; GRAIN_LEN]), write: 0, ph: 0.0 }
    }

    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let read = (self.write as f32 - delay + GRAIN_LEN as f32) % GRAIN_LEN as f32;
        let idx = read as usize;
        let frac = read - idx as f32;
        let a = self.buf[idx % GRAIN_LEN];
        let b = self.buf[(idx + 1) % GRAIN_LEN];
        a + (b - a) * frac
    }

    /// Push one input sample and read back at the given pitch ratio.
    #[inline]
    fn tick(&mut self, input: f32, ratio: f32) -> f32 {
        self.buf[self.write] = input;
        self.write = (self.write + 1) % GRAIN_LEN;
        // Read pointer drifts so the window plays at `ratio` speed.
        self.ph = (self.ph + (1.0 - ratio) / GRAIN_WIN).rem_euclid(1.0);
        let t2 = (self.ph + 0.5).fract();
        let g1 = 0.5 - 0.5 * (TAU * self.ph).cos();
        let g2 = 0.5 - 0.5 * (TAU * t2).cos();
        let d1 = self.ph * GRAIN_WIN + 1.0;
        let d2 = t2 * GRAIN_WIN + 1.0;
        self.read(d1) * g1 + self.read(d2) * g2
    }
}

pub struct PitchShift {
    grain: Grain,
    semis: Smoothed,
    fine: Smoothed,
    mix: Smoothed,
}

impl PitchShift {
    pub fn new() -> Self {
        Self {
            grain: Grain::new(),
            semis: Smoothed::new(0.0),
            fine: Smoothed::new(0.0),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.semis.set_target(value.clamp(-24.0, 24.0)),
            1 => self.fine.set_target(value.clamp(-100.0, 100.0)),
            2 => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        for i in 0..frames {
            let semis = self.semis.tick(ctx.smooth_k);
            let fine = self.fine.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let ratio = 2.0f32.powf((semis + fine * 0.01) / 12.0);
            let dry = input.map_or(0.0, |b| b.mono(i));
            let wet = self.grain.tick(dry, ratio);
            out.data[0][i] = dry + (wet - dry) * mix;
        }
    }
}

impl Default for PitchShift {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Frequency shifter (mono): Hilbert transform (Niemitalo polyphase allpass
// pair) + single-sideband heterodyne.
// params: 0 shift (Hz), 1 mix
// ---------------------------------------------------------------------------

const HIL_A: [f32; 4] = [0.6923878, 0.9360654322959, 0.9882295226860, 0.9987488452737];
const HIL_B: [f32; 4] = [0.4021921162426, 0.8561710882420, 0.9722909545651, 0.9952884791278];

#[derive(Clone, Copy, Default)]
struct Ap2 {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Ap2 {
    #[inline]
    fn process(&mut self, x: f32, a: f32) -> f32 {
        // Second-order allpass: y0 = x2 + (x0 - y2) * a
        let y0 = self.x2 + (x - self.y2) * a;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }
}

pub struct FreqShift {
    chain_a: [Ap2; 4],
    chain_b: [Ap2; 4],
    delay_in: f32,
    osc: f32,
    shift: Smoothed,
    mix: Smoothed,
}

impl FreqShift {
    pub fn new() -> Self {
        Self {
            chain_a: [Ap2::default(); 4],
            chain_b: [Ap2::default(); 4],
            delay_in: 0.0,
            osc: 0.0,
            shift: Smoothed::new(0.0),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.shift.set_target(value.clamp(-1000.0, 1000.0)),
            1 => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        for i in 0..frames {
            let shift = self.shift.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));

            let mut a = dry;
            for (s, &c) in self.chain_a.iter_mut().zip(HIL_A.iter()) {
                a = s.process(a, c);
            }
            let mut b = self.delay_in; // B path operates on the 1-sample-delayed input
            for (s, &c) in self.chain_b.iter_mut().zip(HIL_B.iter()) {
                b = s.process(b, c);
            }
            self.delay_in = dry;

            // a ≈ in-phase, b ≈ quadrature (90° apart across the band).
            self.osc = (self.osc + shift * ctx.inv_sample_rate).rem_euclid(1.0);
            let (sn, cs) = (TAU * self.osc).sin_cos();
            let wet = a * cs - b * sn;
            out.data[0][i] = dry + (wet - dry) * mix;
        }
    }
}

impl Default for FreqShift {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shimmer reverb (mono): Schroeder reverb with an octave-up pitch-shifted
// feedback path for the classic ambient sheen.
// params: 0 size, 1 shimmer, 2 tone, 3 mix
// ---------------------------------------------------------------------------

const COMB_LENS: [usize; 4] = [1557, 1617, 1491, 1422];
const AP_LENS: [usize; 2] = [225, 556];
const COMB_MAX: usize = 2048;
const AP_MAX: usize = 1024;

struct Comb {
    buf: [f32; COMB_MAX],
    idx: usize,
    damp: f32,
    len: usize,
}
impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: [0.0; COMB_MAX], idx: 0, damp: 0.0, len }
    }
    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp_k: f32) -> f32 {
        let y = self.buf[self.idx];
        self.damp = y * (1.0 - damp_k) + self.damp * damp_k;
        self.buf[self.idx] = undenorm(x + self.damp * feedback);
        self.idx = (self.idx + 1) % self.len;
        y
    }
}

struct Allpass {
    buf: [f32; AP_MAX],
    idx: usize,
    len: usize,
}
impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: [0.0; AP_MAX], idx: 0, len }
    }
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let buffered = self.buf[self.idx];
        let y = -x + buffered;
        self.buf[self.idx] = undenorm(x + buffered * 0.5);
        self.idx = (self.idx + 1) % self.len;
        y
    }
}

pub struct Shimmer {
    combs: [Comb; 4],
    aps: [Allpass; 2],
    grain: Grain,
    size: Smoothed,
    shimmer: Smoothed,
    tone: Smoothed,
    mix: Smoothed,
}

impl Shimmer {
    pub fn new() -> Self {
        Self {
            combs: [
                Comb::new(COMB_LENS[0]),
                Comb::new(COMB_LENS[1]),
                Comb::new(COMB_LENS[2]),
                Comb::new(COMB_LENS[3]),
            ],
            aps: [Allpass::new(AP_LENS[0]), Allpass::new(AP_LENS[1])],
            grain: Grain::new(),
            size: Smoothed::new(0.7),
            shimmer: Smoothed::new(0.4),
            tone: Smoothed::new(0.5),
            mix: Smoothed::new(0.4),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.size.set_target(value.clamp(0.0, 1.0)),
            1 => self.shimmer.set_target(value.clamp(0.0, 1.0)),
            2 => self.tone.set_target(value.clamp(0.0, 1.0)),
            3 => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        let mut shimmer_fb = 0.0f32;
        for i in 0..frames {
            let size = self.size.tick(ctx.smooth_k);
            let shimmer = self.shimmer.tick(ctx.smooth_k);
            let tone = self.tone.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));

            let feedback = 0.7 + size * 0.28;
            let damp = 1.0 - tone;
            let inject = dry + shimmer_fb * shimmer;
            let mut acc = 0.0;
            for c in &mut self.combs {
                acc += c.tick(inject * 0.25, feedback, damp);
            }
            for ap in &mut self.aps {
                acc = ap.tick(acc);
            }
            // Octave-up shimmer feedback for the next sample.
            shimmer_fb = self.grain.tick(acc, 2.0);
            out.data[0][i] = dry + (acc - dry) * mix;
        }
    }
}

impl Default for Shimmer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Vocoder (mono): a band-pass bank tracks the modulator's spectral envelope
// and imposes it on the carrier.
// inputs: carrier, modulator
// params: 0 formant shift (semitones), 1 res, 2 mix
// ---------------------------------------------------------------------------

const VOC_BANDS: usize = 14;

pub struct Vocoder {
    carrier: [Svf; VOC_BANDS],
    modulator: [Svf; VOC_BANDS],
    env: [f32; VOC_BANDS],
    formant: Smoothed,
    res: Smoothed,
    mix: Smoothed,
}

impl Vocoder {
    pub fn new() -> Self {
        Self {
            carrier: [Svf::default(); VOC_BANDS],
            modulator: [Svf::default(); VOC_BANDS],
            env: [0.0; VOC_BANDS],
            formant: Smoothed::new(0.0),
            res: Smoothed::new(5.0),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.formant.set_target(value.clamp(-12.0, 12.0)),
            1 => self.res.set_target(value.clamp(1.0, 10.0)),
            2 => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn band_hz(band: usize) -> f32 {
        // Log-spaced 120 Hz .. 7 kHz.
        let t = band as f32 / (VOC_BANDS - 1) as f32;
        120.0 * (7000.0f32 / 120.0).powf(t)
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        carrier: Option<&PortBuffer>,
        modulator: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let _ = propagate_channels(&[carrier, modulator]);
        let env_k = 1.0 - (-1.0 / (0.010 * ctx.sample_rate)).exp();
        for s in self.carrier.iter_mut().chain(self.modulator.iter_mut()) {
            s.undenorm();
        }
        for i in 0..frames {
            let formant = self.formant.tick(ctx.smooth_k);
            let res = self.res.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let car = carrier.map_or(0.0, |b| b.mono(i));
            let modu = modulator.map_or(0.0, |b| b.mono(i));
            let shift = 2.0f32.powf(formant / 12.0);
            let mut wet = 0.0;
            for band in 0..VOC_BANDS {
                let f = Self::band_hz(band).clamp(20.0, ctx.sample_rate * 0.45);
                let cf = (f * shift).clamp(20.0, ctx.sample_rate * 0.45);
                let mc = SvfCoeffs::new(f, res, ctx.sample_rate);
                let cc = SvfCoeffs::new(cf, res, ctx.sample_rate);
                let m = self.modulator[band].tick(modu, &mc).bp;
                self.env[band] += env_k * (m.abs() - self.env[band]);
                let c = self.carrier[band].tick(car, &cc).bp;
                wet += c * self.env[band] * 2.0;
            }
            let wet = tanh_pade(wet / AUDIO_PEAK) * AUDIO_PEAK;
            out.data[0][i] = car + (wet - car) * mix;
        }
    }
}

impl Default for Vocoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn sine_buf(phase: &mut f32, freq: f32, sr: f32) -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        for s in b.data[0].iter_mut() {
            *s = (TAU * *phase).sin() * AUDIO_PEAK;
            *phase = (*phase + freq / sr).fract();
        }
        b
    }

    fn assert_finite(out: &PortBuffer, frames: usize, name: &str) {
        for &s in out.data[0][..frames].iter() {
            assert!(s.is_finite(), "{name} produced non-finite output");
            assert!(s.abs() < 100.0, "{name} blew up: {s}");
        }
    }

    #[test]
    fn feedback_effects_stay_finite() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ph = 0.0;
        let mut tape = TapeDelay::new();
        let mut shim = Shimmer::new();
        let mut ps = PitchShift::new();
        let mut fs = FreqShift::new();
        let mut voc = Vocoder::new();
        tape.set_param(1, 0.95); // max feedback
        shim.set_param(1, 1.0); // max shimmer
        ps.set_param(0, 12.0); // octave up
        fs.set_param(0, 250.0);
        // Run a few hundred blocks so feedback paths settle.
        for _ in 0..400 {
            let inp = sine_buf(&mut ph, 220.0, 48_000.0);
            let mut out = PortBuffer::silent();
            tape.process(&ctx, Some(&inp), &mut out, BLOCK);
            assert_finite(&out, BLOCK, "tapedelay");
            shim.process(&ctx, Some(&inp), &mut out, BLOCK);
            assert_finite(&out, BLOCK, "shimmer");
            ps.process(&ctx, Some(&inp), &mut out, BLOCK);
            assert_finite(&out, BLOCK, "pitchshift");
            fs.process(&ctx, Some(&inp), &mut out, BLOCK);
            assert_finite(&out, BLOCK, "freqshift");
            voc.process(&ctx, Some(&inp), Some(&inp), &mut out, BLOCK);
            assert_finite(&out, BLOCK, "vocoder");
        }
    }
}
