//! Extra sound sources: sub oscillator, supersaw, Karplus–Strong pluck,
//! wavefolder, chord oscillator, and a modal resonator. All take a V/oct
//! input (0 V = C4) where pitched and follow the same channel-propagation
//! rules as the core VCO.

use crate::buffer::{propagate_channels, PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use core::f32::consts::TAU;
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK, GATE_THRESHOLD};
use rack_dsp::{tanh_pade, Smoothed};

const DEFAULT_PITCH: f32 = 0.0; // C4

#[inline]
fn osc_wave(phase: f32, wave_is_sine: bool) -> f32 {
    if wave_is_sine {
        (TAU * phase).sin()
    } else if phase < 0.5 {
        1.0
    } else {
        -1.0
    }
}

// ---------------------------------------------------------------------------
// Sub Oscillator: fundamental plus one and two octaves below.
// params: 0 fund, 1 -1 oct, 2 -2 oct, 3 wave (0 square / 1 sine)
// ---------------------------------------------------------------------------

pub struct SubOsc {
    ph0: [f32; MAX_CHANNELS],
    ph1: [f32; MAX_CHANNELS],
    ph2: [f32; MAX_CHANNELS],
    pitch: Smoothed,
    fund: Smoothed,
    l1: Smoothed,
    l2: Smoothed,
    sine: bool,
}

impl SubOsc {
    pub fn new() -> Self {
        Self {
            ph0: [0.0; MAX_CHANNELS],
            ph1: [0.0; MAX_CHANNELS],
            ph2: [0.0; MAX_CHANNELS],
            pitch: Smoothed::new(DEFAULT_PITCH),
            fund: Smoothed::new(0.3),
            l1: Smoothed::new(0.7),
            l2: Smoothed::new(0.4),
            sine: false,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.pitch.set_target(value),
            1 => self.fund.set_target(value.clamp(0.0, 1.0)),
            2 => self.l1.set_target(value.clamp(0.0, 1.0)),
            3 => self.l2.set_target(value.clamp(0.0, 1.0)),
            4 => self.sine = value as u32 == 1,
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        let mut pitch = [0.0f32; BLOCK];
        let mut fund = [0.0f32; BLOCK];
        let mut l1 = [0.0f32; BLOCK];
        let mut l2 = [0.0f32; BLOCK];
        for i in 0..frames {
            pitch[i] = self.pitch.tick(ctx.smooth_k);
            fund[i] = self.fund.tick(ctx.smooth_k);
            l1[i] = self.l1.tick(ctx.smooth_k);
            l2[i] = self.l2.tick(ctx.smooth_k);
        }
        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let v = pitch[i] + cv.map_or(0.0, |c| c[i]);
                let dt = (voct_to_hz(v) * ctx.inv_sample_rate).min(0.45);
                self.ph0[ch] = (self.ph0[ch] + dt).fract();
                self.ph1[ch] = (self.ph1[ch] + dt * 0.5).fract();
                self.ph2[ch] = (self.ph2[ch] + dt * 0.25).fract();
                let s = fund[i] * osc_wave(self.ph0[ch], self.sine)
                    + l1[i] * osc_wave(self.ph1[ch], self.sine)
                    + l2[i] * osc_wave(self.ph2[ch], self.sine);
                out.data[ch][i] = (s * 0.5).clamp(-1.5, 1.5) * AUDIO_PEAK;
            }
        }
    }
}

impl Default for SubOsc {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Supersaw: seven detuned saws (naive — bright by design).
// params: 0 pitch, 1 detune, 2 mix (center vs sides)
// ---------------------------------------------------------------------------

const SAWS: usize = 7;
// Symmetric detune spread in semitones, scaled by the detune knob.
const SPREAD: [f32; SAWS] = [-1.0, -0.7, -0.4, 0.0, 0.4, 0.7, 1.0];

pub struct Supersaw {
    phase: [[f32; SAWS]; MAX_CHANNELS],
    pitch: Smoothed,
    detune: Smoothed,
    mix: Smoothed,
}

impl Supersaw {
    pub fn new() -> Self {
        Self {
            phase: [[0.0; SAWS]; MAX_CHANNELS],
            pitch: Smoothed::new(DEFAULT_PITCH),
            detune: Smoothed::new(0.2),
            mix: Smoothed::new(0.7),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.pitch.set_target(value),
            1 => self.detune.set_target(value.clamp(0.0, 1.0)),
            2 => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let pitch = self.pitch.tick(ctx.smooth_k);
                let detune = self.detune.tick(ctx.smooth_k);
                let mix = self.mix.tick(ctx.smooth_k);
                let base = pitch + cv.map_or(0.0, |c| c[i]);
                let mut center = 0.0;
                let mut sides = 0.0;
                for s in 0..SAWS {
                    // Detune up to ±0.5 semitone (×detune) per spread step.
                    let v = base + SPREAD[s] * detune * (0.5 / 12.0);
                    let dt = (voct_to_hz(v) * ctx.inv_sample_rate).min(0.45);
                    let ph = (self.phase[ch][s] + dt).fract();
                    self.phase[ch][s] = ph;
                    let saw = 2.0 * ph - 1.0;
                    if s == SAWS / 2 {
                        center = saw;
                    } else {
                        sides += saw;
                    }
                }
                let s = center * (1.0 - mix) + (sides / (SAWS - 1) as f32) * mix * 2.0;
                out.data[ch][i] = (s * 0.6).clamp(-1.5, 1.5) * AUDIO_PEAK;
            }
        }
    }
}

impl Default for Supersaw {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Karplus–Strong pluck. A trigger fills the delay line with noise; the loop
// low-passes and decays it into a plucked-string tone.
// params: 0 pitch, 1 decay, 2 tone (damping)
// ---------------------------------------------------------------------------

const KS_MAX: usize = 2048; // lowest pitch ~23 Hz at 48 kHz

pub struct Pluck {
    buf: Box<[[f32; KS_MAX]; MAX_CHANNELS]>,
    pos: [usize; MAX_CHANNELS],
    len: [usize; MAX_CHANNELS],
    prev_trig: [bool; MAX_CHANNELS],
    rng: u32,
    pitch: Smoothed,
    decay: Smoothed,
    tone: Smoothed,
}

impl Pluck {
    pub fn new() -> Self {
        Self {
            buf: Box::new([[0.0; KS_MAX]; MAX_CHANNELS]),
            pos: [0; MAX_CHANNELS],
            len: [128; MAX_CHANNELS],
            prev_trig: [false; MAX_CHANNELS],
            rng: 0x1234_5678,
            pitch: Smoothed::new(DEFAULT_PITCH),
            decay: Smoothed::new(0.6),
            tone: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.pitch.set_target(value),
            1 => self.decay.set_target(value.clamp(0.0, 1.0)),
            2 => self.tone.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        trig: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[voct, trig]).max(1);
        out.channels = channels;
        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let tg = trig.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let pitch = self.pitch.tick(ctx.smooth_k);
                let decay = self.decay.tick(ctx.smooth_k);
                let tone = self.tone.tick(ctx.smooth_k);

                // Retune the loop length from pitch.
                let v = pitch + cv.map_or(0.0, |c| c[i]);
                let hz = voct_to_hz(v).max(20.0);
                let len = ((ctx.sample_rate / hz) as usize).clamp(2, KS_MAX - 1);
                self.len[ch] = len;

                // Trigger: excite with a fresh noise burst.
                let high = tg.map_or(false, |t| t[i] >= GATE_THRESHOLD);
                if high && !self.prev_trig[ch] {
                    for j in 0..len {
                        self.buf[ch][j] = self.noise();
                    }
                    self.pos[ch] = 0;
                }
                self.prev_trig[ch] = high;

                let len = self.len[ch];
                let pos = self.pos[ch];
                let next = (pos + 1) % len;
                let y = self.buf[ch][pos];
                // Damping low-pass: brighter as tone rises.
                let damp = 0.5 - 0.2 * (1.0 - tone);
                let filtered = y * (1.0 - damp) + self.buf[ch][next] * damp;
                self.buf[ch][pos] = filtered * (0.9 + 0.099 * decay);
                self.pos[ch] = next;
                out.data[ch][i] = y * AUDIO_PEAK;
            }
        }
    }
}

impl Default for Pluck {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Wavefolder: west-coast triangle folding with bias and drive.
// params: 0 fold, 1 symmetry (bias), 2 mix
// ---------------------------------------------------------------------------

pub struct Wavefold {
    fold: Smoothed,
    bias: Smoothed,
    mix: Smoothed,
}

impl Wavefold {
    pub fn new() -> Self {
        Self { fold: Smoothed::new(1.0), bias: Smoothed::new(0.0), mix: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.fold.set_target(value.clamp(1.0, 8.0)),
            1 => self.bias.set_target(value.clamp(-1.0, 1.0)),
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
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let fold = self.fold.tick(ctx.smooth_k);
                let bias = self.bias.tick(ctx.smooth_k);
                let mix = self.mix.tick(ctx.smooth_k);
                let dry = in_data.map_or(0.0, |d| d[i]);
                // Normalise to ±1, scale by fold, add bias, then triangle-fold.
                let mut x = dry / AUDIO_PEAK * fold + bias;
                // Fold into [-1, 1] by reflection.
                for _ in 0..4 {
                    if x > 1.0 {
                        x = 2.0 - x;
                    } else if x < -1.0 {
                        x = -2.0 - x;
                    } else {
                        break;
                    }
                }
                let folded = (TAU * 0.25 * x).sin(); // round the corners
                let wet = folded * AUDIO_PEAK;
                out.data[ch][i] = dry + (wet - dry) * mix;
            }
        }
    }
}

impl Default for Wavefold {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Chord oscillator: one V/oct in, a voiced chord out (poly-free, summed mono).
// params: 0 pitch, 1 chord type, 2 wave (0 saw / 1 sine), 3 detune
// ---------------------------------------------------------------------------

const N_CHORD: usize = 4;
// Semitone offsets per chord type: maj, min, dom7, maj7, min7, sus4.
const CHORDS: [[f32; N_CHORD]; 6] = [
    [0.0, 4.0, 7.0, 12.0],
    [0.0, 3.0, 7.0, 12.0],
    [0.0, 4.0, 7.0, 10.0],
    [0.0, 4.0, 7.0, 11.0],
    [0.0, 3.0, 7.0, 10.0],
    [0.0, 5.0, 7.0, 12.0],
];

pub struct ChordOsc {
    phase: [[f32; N_CHORD]; MAX_CHANNELS],
    pitch: Smoothed,
    detune: Smoothed,
    chord: usize,
    sine: bool,
}

impl ChordOsc {
    pub fn new() -> Self {
        Self {
            phase: [[0.0; N_CHORD]; MAX_CHANNELS],
            pitch: Smoothed::new(DEFAULT_PITCH),
            detune: Smoothed::new(0.02),
            chord: 0,
            sine: false,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.pitch.set_target(value),
            1 => self.chord = (value as usize).min(CHORDS.len() - 1),
            2 => self.sine = value as u32 == 1,
            3 => self.detune.set_target(value.clamp(0.0, 0.2)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        let chord = CHORDS[self.chord];
        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let pitch = self.pitch.tick(ctx.smooth_k);
                let detune = self.detune.tick(ctx.smooth_k);
                let base = pitch + cv.map_or(0.0, |c| c[i]);
                let mut s = 0.0;
                for n in 0..N_CHORD {
                    let v = base + chord[n] / 12.0 + detune * (n as f32 - 1.5) * 0.05;
                    let dt = (voct_to_hz(v) * ctx.inv_sample_rate).min(0.45);
                    let ph = (self.phase[ch][n] + dt).fract();
                    self.phase[ch][n] = ph;
                    s += if self.sine { (TAU * ph).sin() } else { 2.0 * ph - 1.0 };
                }
                out.data[ch][i] = (s / N_CHORD as f32 * 0.9) * AUDIO_PEAK;
            }
        }
    }
}

impl Default for ChordOsc {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Resonator: a small bank of tuned, high-Q band-passes excited by the input.
// params: 0 pitch, 1 structure (harmonic↔inharmonic), 2 brightness, 3 decay
// ---------------------------------------------------------------------------

const N_MODES: usize = 4;

pub struct Resonator {
    svf: [[Svf; N_MODES]; MAX_CHANNELS],
    pitch: Smoothed,
    structure: Smoothed,
    bright: Smoothed,
    decay: Smoothed,
}

impl Resonator {
    pub fn new() -> Self {
        Self {
            svf: [[Svf::default(); N_MODES]; MAX_CHANNELS],
            pitch: Smoothed::new(DEFAULT_PITCH),
            structure: Smoothed::new(0.5),
            bright: Smoothed::new(0.5),
            decay: Smoothed::new(0.7),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.pitch.set_target(value),
            1 => self.structure.set_target(value.clamp(0.0, 1.0)),
            2 => self.bright.set_target(value.clamp(0.0, 1.0)),
            3 => self.decay.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        voct: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, voct]).max(1);
        out.channels = channels;
        let structure = self.structure.current();
        let bright = self.bright.current();
        let decay = self.decay.current();
        for _ in 0..frames {
            self.structure.tick(ctx.smooth_k);
            self.bright.tick(ctx.smooth_k);
            self.decay.tick(ctx.smooth_k);
        }
        let q = 2.0 + decay * 198.0; // higher decay → longer ring
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            for s in &mut self.svf[ch] {
                s.undenorm();
            }
            for i in 0..frames {
                let pitch = self.pitch.tick(ctx.smooth_k);
                let v = pitch + cv.map_or(0.0, |c| c[i]);
                let f0 = voct_to_hz(v).clamp(20.0, ctx.sample_rate * 0.45);
                let exc = in_data.map_or(0.0, |d| d[i]);
                let mut acc = 0.0;
                for m in 0..N_MODES {
                    // Harmonic ratio bent toward inharmonic by `structure`.
                    let harm = (m + 1) as f32;
                    let inharm = harm.powf(1.0 + structure * 0.6);
                    let ratio = harm + (inharm - harm) * structure;
                    let f = (f0 * ratio).min(ctx.sample_rate * 0.45);
                    let c = SvfCoeffs::new(f, q, ctx.sample_rate);
                    let bp = self.svf[ch][m].tick(exc, &c).bp;
                    // Brightness tilts gain toward higher modes.
                    let g = 1.0 - (1.0 - bright) * (m as f32 / N_MODES as f32);
                    acc += bp * g;
                }
                out.data[ch][i] = tanh_pade(acc / N_MODES as f32) * AUDIO_PEAK;
            }
        }
    }
}

impl Default for Resonator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn voct(volts: f32) -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        b.data[0] = [volts; BLOCK];
        b
    }

    fn check(out: &PortBuffer, name: &str) {
        for &s in out.data[0][..BLOCK].iter() {
            assert!(s.is_finite(), "{name} non-finite");
            assert!(s.abs() < 100.0, "{name} blew up: {s}");
        }
    }

    #[test]
    fn new_sources_make_finite_audio() {
        let ctx = ProcessCtx::new(48_000.0);
        let cv = voct(0.0);
        let mut trig = PortBuffer::silent();
        trig.channels = 1;
        trig.data[0][0] = 10.0; // pluck excite
        let mut sub = SubOsc::new();
        let mut sup = Supersaw::new();
        let mut pluck = Pluck::new();
        let mut fold = Wavefold::new();
        let mut chord = ChordOsc::new();
        let mut reso = Resonator::new();
        fold.set_param(0, 6.0);
        for _ in 0..200 {
            let mut out = PortBuffer::silent();
            sub.process(&ctx, Some(&cv), &mut out, BLOCK);
            check(&out, "subosc");
            sup.process(&ctx, Some(&cv), &mut out, BLOCK);
            check(&out, "supersaw");
            pluck.process(&ctx, Some(&cv), Some(&trig), &mut out, BLOCK);
            check(&out, "pluck");
            fold.process(&ctx, Some(&out.clone()), &mut out, BLOCK);
            check(&out, "wavefold");
            chord.process(&ctx, Some(&cv), &mut out, BLOCK);
            check(&out, "chordosc");
            reso.process(&ctx, Some(&cv), Some(&cv), &mut out, BLOCK);
            check(&out, "resonator");
        }
    }
}
