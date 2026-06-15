//! Timbre tools: ring modulator, bit crusher, Karplus-Strong comb (plucked
//! string), and a vowel formant filter.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{bitcrush as bp, comb as cp, formant as fp};
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK};
use rack_dsp::{undenorm, Smoothed};

// ---------------------------------------------------------------------------
// Ring modulator
// ---------------------------------------------------------------------------

pub struct RingMod;

impl RingMod {
    pub fn new() -> Self {
        Self
    }
    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        a: Option<&PortBuffer>,
        b: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[a, b]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let ad = a.map(|x| x.channel_or_broadcast(ch));
            let bd = b.map(|x| x.channel_or_broadcast(ch));
            for i in 0..frames {
                // Product normalized so two ±5 V signals stay ±5 V.
                out.data[ch][i] =
                    ad.map_or(0.0, |d| d[i]) * bd.map_or(0.0, |d| d[i]) / AUDIO_PEAK;
            }
        }
    }
}

impl Default for RingMod {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Bit crusher / sample-rate reducer
// ---------------------------------------------------------------------------

pub struct BitCrush {
    held: [f32; MAX_CHANNELS],
    counter: [u32; MAX_CHANNELS],
    bits: Smoothed,
    downsample: Smoothed,
    mix: Smoothed,
}

impl BitCrush {
    pub fn new() -> Self {
        Self {
            held: [0.0; MAX_CHANNELS],
            counter: [0; MAX_CHANNELS],
            bits: Smoothed::new(8.0),
            downsample: Smoothed::new(1.0),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            bp::BITS => self.bits.set_target(value.clamp(1.0, 16.0)),
            bp::DOWNSAMPLE => self.downsample.set_target(value.clamp(1.0, 64.0)),
            bp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        let bits = self.bits.current();
        let down = self.downsample.current().round().max(1.0) as u32;
        let mix = self.mix.current();
        for _ in 0..frames {
            self.bits.tick(ctx.smooth_k);
            self.downsample.tick(ctx.smooth_k);
            self.mix.tick(ctx.smooth_k);
        }
        let levels = 2f32.powf(bits);

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let dry = in_data.map_or(0.0, |d| d[i]);
                if self.counter[ch] == 0 {
                    // Quantize to exactly `2^bits` levels over the ±5 V range.
                    let norm = (dry / AUDIO_PEAK).clamp(-1.0, 1.0);
                    let unipolar = norm * 0.5 + 0.5; // 0..1
                    let q = (unipolar * (levels - 1.0)).round() / (levels - 1.0);
                    self.held[ch] = (q * 2.0 - 1.0) * AUDIO_PEAK;
                }
                self.counter[ch] = (self.counter[ch] + 1) % down;
                out.data[ch][i] = dry + (self.held[ch] - dry) * mix;
            }
        }
    }
}

impl Default for BitCrush {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Karplus-Strong comb (plucked string)
// ---------------------------------------------------------------------------

const KS_MAX: usize = 4800; // lowest pitch ~10 Hz at 48 kHz

pub struct Comb {
    buf: Box<[f32; KS_MAX]>,
    pos: usize,
    damp_state: f32,
    pitch: Smoothed,
    decay: Smoothed,
    damp: Smoothed,
}

impl Comb {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0.0; KS_MAX]),
            pos: 0,
            damp_state: 0.0,
            pitch: Smoothed::new(0.0),
            decay: Smoothed::new(0.98),
            damp: Smoothed::new(0.3),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            cp::PITCH => self.pitch.set_target(value),
            cp::DECAY => self.decay.set_target(value.clamp(0.8, 0.999)),
            cp::DAMP => self.damp.set_target(value.clamp(0.0, 1.0)),
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
        out.channels = 1;
        let voct_data = voct.map(|b| b.channel_or_broadcast(0));

        for i in 0..frames {
            let pitch = self.pitch.tick(ctx.smooth_k) + voct_data.map_or(0.0, |c| c[i]);
            let decay = self.decay.tick(ctx.smooth_k);
            let damp = self.damp.tick(ctx.smooth_k);
            let hz = voct_to_hz(pitch).clamp(10.0, ctx.sample_rate * 0.45);
            let delay = (ctx.sample_rate / hz).clamp(2.0, (KS_MAX - 2) as f32);

            // The input excites the string: patch a short noise/impulse burst
            // (or any audio) into it and it rings at the set pitch.
            let exc = input.map_or(0.0, |b| b.mono(i));

            let read = self.pos as f32 - delay;
            let read = if read < 0.0 { read + KS_MAX as f32 } else { read };
            let idx = read as usize;
            let frac = read - idx as f32;
            let a = self.buf[idx % KS_MAX];
            let b = self.buf[(idx + 1) % KS_MAX];
            let delayed = a + (b - a) * frac;

            // Damping low-pass in the feedback path (string brightness).
            self.damp_state += (1.0 - damp) * (delayed - self.damp_state);
            let fed = exc + self.damp_state * decay;
            self.buf[self.pos] = undenorm(fed);
            self.pos = (self.pos + 1) % KS_MAX;

            out.data[0][i] = delayed;
        }
    }
}

impl Default for Comb {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Formant (vowel) filter — three parallel band-passes per vowel
// ---------------------------------------------------------------------------

/// Formant centre frequencies (Hz) for A E I O U (typical male vowels).
const VOWELS: [[f32; 3]; 5] = [
    [800.0, 1150.0, 2900.0],  // A
    [400.0, 1600.0, 2700.0],  // E
    [350.0, 1700.0, 2700.0],  // I
    [450.0, 800.0, 2830.0],   // O
    [325.0, 700.0, 2530.0],   // U
];

pub struct Formant {
    filters: [Svf; 3],
    vowel: usize,
    res: Smoothed,
}

impl Formant {
    pub fn new() -> Self {
        Self { filters: [Svf::default(); 3], vowel: 0, res: Smoothed::new(8.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            fp::VOWEL => self.vowel = (value as usize).min(VOWELS.len() - 1),
            fp::RES => self.res.set_target(value.clamp(2.0, 20.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        vowel_cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let cv = vowel_cv.map(|b| b.channel_or_broadcast(0));
        // CV (0-10 V) can scan the vowel; otherwise use the knob.
        let vowel = cv
            .map(|c| ((c[0] / 10.0).clamp(0.0, 1.0) * 4.0).round() as usize)
            .unwrap_or(self.vowel)
            .min(VOWELS.len() - 1);
        let res = self.res.current();
        for _ in 0..frames {
            self.res.tick(ctx.smooth_k);
        }
        let coeffs: [SvfCoeffs; 3] =
            core::array::from_fn(|f| SvfCoeffs::new(VOWELS[vowel][f], res, ctx.sample_rate));
        // Higher formants are progressively quieter, as in real voices.
        let gains = [1.0, 0.6, 0.35];

        for i in 0..frames {
            let x = input.map_or(0.0, |b| b.mono(i));
            let mut y = 0.0;
            for f in 0..3 {
                y += self.filters[f].tick(x, &coeffs[f]).bp * gains[f];
            }
            out.data[0][i] = y;
        }
        for f in &mut self.filters {
            f.undenorm();
        }
    }
}

impl Default for Formant {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn ringmod_creates_sidebands() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut rm = RingMod::new();
        let mut a = PortBuffer::silent();
        let mut b = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        // Two sines multiplied → output at sum/difference, zero DC.
        let (mut pa, mut pb) = (0.0f32, 0.0f32);
        let (da, db) = (200.0 / 48_000.0, 300.0 / 48_000.0);
        let mut sum = 0.0f64;
        let mut n = 0;
        for _ in 0..500 {
            for i in 0..BLOCK {
                a.data[0][i] = (core::f32::consts::TAU * pa).sin() * AUDIO_PEAK;
                b.data[0][i] = (core::f32::consts::TAU * pb).sin() * AUDIO_PEAK;
                pa += da;
                pb += db;
                if pa >= 1.0 { pa -= 1.0; }
                if pb >= 1.0 { pb -= 1.0; }
            }
            rm.process(&ctx, Some(&a), Some(&b), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                assert!(s.abs() <= AUDIO_PEAK + 0.1);
                sum += s as f64;
                n += 1;
            }
        }
        assert!((sum / n as f64).abs() < 0.1, "ringmod output not zero-mean");
    }

    #[test]
    fn bitcrush_quantizes() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut bc = BitCrush::new();
        bc.bits.set_immediate(2.0); // 4 levels — heavy quantization
        bc.mix.set_immediate(1.0);
        let mut input = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        let mut distinct = std::collections::HashSet::new();
        let mut phase = 0.0f32;
        for _ in 0..200 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += 100.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            bc.process(&ctx, Some(&input), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                distinct.insert((s * 1000.0).round() as i32);
            }
        }
        // 2 bits → exactly 4 quantization levels.
        assert!(distinct.len() <= 4, "expected 4 levels, got {}", distinct.len());
    }

    #[test]
    fn karplus_rings_after_excitation_then_decays() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut comb = Comb::new();
        comb.pitch.set_immediate(0.0); // C4
        comb.decay.set_immediate(0.99);
        let mut out = PortBuffer::silent();

        // Excite with a one-block noise burst into the input.
        let mut burst = PortBuffer::silent();
        let mut rng = 0xbeefu32;
        for i in 0..BLOCK {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            burst.data[0][i] = (rng as f32 / u32::MAX as f32 * 2.0 - 1.0) * AUDIO_PEAK;
        }
        comb.process(&ctx, Some(&burst), None, &mut out, BLOCK);

        let silent = PortBuffer::silent();
        let energy = |comb: &mut Comb, out: &mut PortBuffer| {
            let mut e = 0.0f64;
            for _ in 0..20 {
                comb.process(&ctx, Some(&silent), None, out, BLOCK);
                for &s in &out.data[0][..BLOCK] {
                    assert!(s.is_finite());
                    e += (s as f64).powi(2);
                }
            }
            e
        };
        let early = energy(&mut comb, &mut out);
        for _ in 0..200 {
            comb.process(&ctx, Some(&silent), None, &mut out, BLOCK);
        }
        let late = energy(&mut comb, &mut out);
        assert!(early > 1.0, "string didn't ring: {early}");
        assert!(late < early * 0.7, "string didn't decay: {early} -> {late}");
    }

    #[test]
    fn formant_passes_a_band_and_vowels_differ() {
        let ctx = ProcessCtx::new(48_000.0);
        // Drive with a bright saw; different vowels emphasize different bands,
        // so their output energy should differ.
        let render = |vowel: u32| {
            let mut f = Formant::new();
            f.set_param(fp::VOWEL, vowel as f32);
            let mut input = PortBuffer::silent();
            let mut out = PortBuffer::silent();
            let mut phase = 0.0f32;
            let mut e = 0.0f64;
            for _ in 0..500 {
                for i in 0..BLOCK {
                    input.data[0][i] = (2.0 * phase - 1.0) * AUDIO_PEAK;
                    phase += 120.0 / 48_000.0;
                    if phase >= 1.0 { phase -= 1.0; }
                }
                f.process(&ctx, Some(&input), None, &mut out, BLOCK);
                for &s in &out.data[0][..BLOCK] {
                    assert!(s.is_finite());
                    e += (s as f64).powi(2);
                }
            }
            e
        };
        let a = render(0);
        let i = render(2);
        assert!(a > 0.0 && i > 0.0, "formant produced no output");
        assert!((a - i).abs() / a.max(i) > 0.05, "vowels A and I sound identical");
    }
}
