//! Time-based effects: reverb (Freeverb-style comb/allpass network), chorus
//! (LFO-modulated delay), and phaser (cascaded modulated allpass). All are
//! mono-in (read channel 0) to keep delay-line memory bounded; reverb and
//! chorus produce a stereo pair.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::{chorus as cp, phaser as pp, reverb as rp};
use rack_dsp::{undenorm, Smoothed};

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// Fixed-capacity comb filter with feedback and internal damping.
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0, store: 0.0 }
    }

    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = undenorm(x + self.store * feedback);
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

/// Fixed-capacity Schroeder allpass.
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0 }
    }

    #[inline]
    fn tick(&mut self, x: f32, feedback: f32) -> f32 {
        let buffered = self.buf[self.pos];
        let y = -x + buffered;
        self.buf[self.pos] = undenorm(x + buffered * feedback);
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

// ---------------------------------------------------------------------------
// Reverb (Freeverb, reduced to 4 combs + 2 allpass per channel)
// ---------------------------------------------------------------------------

const COMB_TUNING: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASS_TUNING: [usize; 2] = [556, 441];
const STEREO_SPREAD: usize = 23;

pub struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    aps_l: Vec<Allpass>,
    aps_r: Vec<Allpass>,
    decay: Smoothed,
    mix: Smoothed,
}

impl Reverb {
    pub fn new() -> Self {
        // Tunings are for 44.1 kHz; close enough at 48 kHz.
        Self {
            combs_l: COMB_TUNING.iter().map(|&n| Comb::new(n)).collect(),
            combs_r: COMB_TUNING.iter().map(|&n| Comb::new(n + STEREO_SPREAD)).collect(),
            aps_l: ALLPASS_TUNING.iter().map(|&n| Allpass::new(n)).collect(),
            aps_r: ALLPASS_TUNING.iter().map(|&n| Allpass::new(n + STEREO_SPREAD)).collect(),
            decay: Smoothed::new(0.75),
            mix: Smoothed::new(0.3),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            rp::DECAY => self.decay.set_target(value.clamp(0.0, 0.97)),
            rp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        left: &mut PortBuffer,
        right: &mut PortBuffer,
        frames: usize,
    ) {
        left.channels = 1;
        right.channels = 1;
        const DAMP: f32 = 0.2;

        for i in 0..frames {
            // Comb feedback approaches 1 as decay → 1 (longer tail).
            let feedback = 0.7 + self.decay.tick(ctx.smooth_k) * 0.28;
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));
            let input_gain = 0.015; // Freeverb fixed input scaling

            let mut l = 0.0;
            let mut r = 0.0;
            for c in &mut self.combs_l {
                l += c.tick(dry * input_gain, feedback, DAMP);
            }
            for c in &mut self.combs_r {
                r += c.tick(dry * input_gain, feedback, DAMP);
            }
            for a in &mut self.aps_l {
                l = a.tick(l, 0.5);
            }
            for a in &mut self.aps_r {
                r = a.tick(r, 0.5);
            }
            // Wet is in audio volts already (input was ±5 scaled down then
            // built back up by the network); blend with dry.
            left.data[0][i] = dry + (l * 5.0 - dry) * mix;
            right.data[0][i] = dry + (r * 5.0 - dry) * mix;
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Chorus (dual LFO-modulated delay)
// ---------------------------------------------------------------------------

const CHORUS_MAX: usize = 2048; // ~43 ms at 48 kHz

pub struct Chorus {
    buf: Box<[f32; CHORUS_MAX]>,
    pos: usize,
    lfo_phase: f32,
    rate: Smoothed,
    depth: Smoothed,
    mix: Smoothed,
}

impl Chorus {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0.0; CHORUS_MAX]),
            pos: 0,
            lfo_phase: 0.0,
            rate: Smoothed::new(0.8),
            depth: Smoothed::new(0.5),
            mix: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            cp::RATE => self.rate.set_target(value.clamp(0.05, 5.0)),
            cp::DEPTH => self.depth.set_target(value.clamp(0.0, 1.0)),
            cp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        left: &mut PortBuffer,
        right: &mut PortBuffer,
        frames: usize,
    ) {
        left.channels = 1;
        right.channels = 1;
        // Base delay ~12 ms, modulation up to ~7 ms.
        let base = 0.012 * ctx.sample_rate;
        let len = CHORUS_MAX as f32;

        for i in 0..frames {
            let rate = self.rate.tick(ctx.smooth_k);
            let depth = self.depth.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));

            self.buf[self.pos] = dry;

            let mod_samples = depth * 0.006 * ctx.sample_rate;
            // Two quadrature LFOs for stereo width.
            let lfo_l = (core::f32::consts::TAU * self.lfo_phase).sin();
            let lfo_r = (core::f32::consts::TAU * (self.lfo_phase + 0.25)).sin();
            let wet_l = self.read(base + mod_samples * lfo_l, len);
            let wet_r = self.read(base + mod_samples * lfo_r, len);

            left.data[0][i] = dry + (wet_l - dry) * mix;
            right.data[0][i] = dry + (wet_r - dry) * mix;

            self.pos = (self.pos + 1) % CHORUS_MAX;
            self.lfo_phase += rate * ctx.inv_sample_rate;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }

    #[inline]
    fn read(&self, delay_samples: f32, len: f32) -> f32 {
        let d = delay_samples.clamp(1.0, len - 2.0);
        let read_pos = self.pos as f32 - d;
        let read_pos = if read_pos < 0.0 { read_pos + len } else { read_pos };
        let idx = read_pos as usize;
        let frac = read_pos - idx as f32;
        let a = self.buf[idx % CHORUS_MAX];
        let b = self.buf[(idx + 1) % CHORUS_MAX];
        a + (b - a) * frac
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Phaser (cascaded first-order allpass, LFO-swept)
// ---------------------------------------------------------------------------

const PHASER_STAGES: usize = 6;

pub struct Phaser {
    stages: [f32; PHASER_STAGES],
    lfo_phase: f32,
    rate: Smoothed,
    depth: Smoothed,
    mix: Smoothed,
}

impl Phaser {
    pub fn new() -> Self {
        Self {
            stages: [0.0; PHASER_STAGES],
            lfo_phase: 0.0,
            rate: Smoothed::new(0.5),
            depth: Smoothed::new(0.7),
            mix: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            pp::RATE => self.rate.set_target(value.clamp(0.05, 5.0)),
            pp::DEPTH => self.depth.set_target(value.clamp(0.0, 1.0)),
            pp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
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
            let rate = self.rate.tick(ctx.smooth_k);
            let depth = self.depth.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);
            let dry = input.map_or(0.0, |b| b.mono(i));

            // LFO sweeps the allpass coefficient between two pitches.
            let lfo = 0.5 + 0.5 * (core::f32::consts::TAU * self.lfo_phase).sin();
            let coeff = 0.1 + lfo * depth * 0.85;

            let mut x = dry;
            for s in &mut self.stages {
                let y = -coeff * x + *s;
                *s = undenorm(x + coeff * y);
                x = y;
            }
            out.data[0][i] = dry + (x - dry) * mix;

            self.lfo_phase += rate * ctx.inv_sample_rate;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }
}

impl Default for Phaser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{PortBuffer, BLOCK};
    use rack_dsp::volts::AUDIO_PEAK;

    fn impulse_in() -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.data[0][0] = AUDIO_PEAK;
        b
    }

    #[test]
    fn reverb_produces_a_decaying_stereo_tail() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut rev = Reverb::new();
        rev.mix.set_immediate(1.0);
        let imp = impulse_in();
        let silent = PortBuffer::silent();
        let mut l = PortBuffer::silent();
        let mut r = PortBuffer::silent();

        // Equal-size early vs late windows so the comparison is apples to
        // apples (summing unequal spans would let a quiet long tail "win").
        let mut peak_early = 0.0f32;
        let mut peak_late = 0.0f32;
        let mut stereo_diff = 0.0f64;
        for block in 0..400 {
            let inp = if block == 0 { &imp } else { &silent };
            rev.process(&ctx, Some(inp), &mut l, &mut r, BLOCK);
            for i in 0..BLOCK {
                assert!(l.data[0][i].is_finite() && r.data[0][i].is_finite());
                let a = l.data[0][i].abs();
                if (10..40).contains(&block) {
                    peak_early = peak_early.max(a);
                } else if (360..390).contains(&block) {
                    peak_late = peak_late.max(a);
                }
                stereo_diff += ((l.data[0][i] - r.data[0][i]) as f64).abs();
            }
        }
        assert!(peak_early > 0.0, "no reverb output");
        assert!(peak_late < peak_early * 0.7, "reverb tail not decaying: {peak_early} -> {peak_late}");
        assert!(stereo_diff > 0.0, "reverb output is mono");
    }

    #[test]
    fn chorus_stays_bounded_and_wet_differs_from_dry() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ch = Chorus::new();
        ch.mix.set_immediate(1.0);
        ch.depth.set_immediate(1.0);
        let mut l = PortBuffer::silent();
        let mut r = PortBuffer::silent();
        let mut input = PortBuffer::silent();
        let mut phase = 0.0f32;
        let dt = 220.0 / 48_000.0;
        let mut diff = 0.0f64;
        for _ in 0..2000 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += dt;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
            }
            ch.process(&ctx, Some(&input), &mut l, &mut r, BLOCK);
            for i in 0..BLOCK {
                assert!(l.data[0][i].abs() < 20.0 && r.data[0][i].abs() < 20.0);
                diff += ((l.data[0][i] - r.data[0][i]) as f64).abs();
            }
        }
        assert!(diff > 1.0, "chorus produced no stereo movement");
    }

    #[test]
    fn phaser_is_bounded() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ph = Phaser::new();
        ph.mix.set_immediate(1.0);
        let mut out = PortBuffer::silent();
        let mut input = PortBuffer::silent();
        input.data[0] = [AUDIO_PEAK; BLOCK];
        for _ in 0..2000 {
            ph.process(&ctx, Some(&input), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                assert!(s.is_finite() && s.abs() < 20.0);
            }
        }
    }
}
