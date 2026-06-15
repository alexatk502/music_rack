//! Macro oscillator: one module, several switchable synthesis models in the
//! spirit of Mutable Instruments' Plaits. A MODEL switch picks the engine;
//! the HARMONICS / TIMBRE / MORPH controls (with CV on timbre and morph)
//! re-purpose per model. Two outputs — `main` and a model-dependent `aux`.
//!
//! Models: 0 virtual-analog, 1 wavefolder, 2 2-op FM, 3 additive, 4 chord,
//! 5 particle/noise. Polyphonic: channel count follows the v/oct input.

use crate::buffer::{PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::macro_osc as p;
use rack_dsp::polyblep::{saw_blep, square_blep};
use rack_dsp::svf::{Svf, SvfCoeffs};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK};
use rack_dsp::Smoothed;

const N_MODELS: u32 = 6;
/// Chord interval tables (semitones from the root), chosen by HARMONICS.
const CHORDS: [&[f32]; 5] = [
    &[0.0, 12.0],            // octave
    &[0.0, 7.0, 12.0],       // fifth
    &[0.0, 4.0, 7.0],        // major
    &[0.0, 3.0, 7.0],        // minor
    &[0.0, 4.0, 7.0, 11.0],  // major 7
];

#[inline]
fn sin01(phase: f32) -> f32 {
    (core::f32::consts::TAU * phase).sin()
}

/// Per-channel oscillator state.
#[derive(Clone, Copy)]
struct Voice {
    phase: [f32; 4], // sub-oscillators (chord uses all four)
    mod_phase: f32,
    last_mod: f32,
    filt: Svf,
    rng: u32,
}

impl Voice {
    fn new(seed: u32) -> Self {
        Self { phase: [0.0; 4], mod_phase: 0.0, last_mod: 0.0, filt: Svf::default(), rng: seed }
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

pub struct MacroOsc {
    model: u32,
    pitch: Smoothed,
    harmonics: Smoothed,
    timbre: Smoothed,
    morph: Smoothed,
    voices: [Voice; MAX_CHANNELS],
}

impl MacroOsc {
    pub fn new() -> Self {
        // Distinct RNG seeds per channel so noise voices don't correlate.
        let mut voices = [Voice::new(1); MAX_CHANNELS];
        for (i, v) in voices.iter_mut().enumerate() {
            *v = Voice::new(0x9e37_79b9u32.wrapping_mul(i as u32 + 1) | 1);
        }
        Self {
            model: 0,
            pitch: Smoothed::new(0.75),
            harmonics: Smoothed::new(0.5),
            timbre: Smoothed::new(0.5),
            morph: Smoothed::new(0.5),
            voices,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::MODEL => self.model = (value as u32).min(N_MODELS - 1),
            p::PITCH => self.pitch.set_target(value),
            p::HARMONICS => self.harmonics.set_target(value.clamp(0.0, 1.0)),
            p::TIMBRE => self.timbre.set_target(value.clamp(0.0, 1.0)),
            p::MORPH => self.morph.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        timbre_cv: Option<&PortBuffer>,
        morph_cv: Option<&PortBuffer>,
        main: &mut PortBuffer,
        aux: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        main.channels = channels;
        aux.channels = channels;

        // Smooth knobs once per block into per-sample arrays.
        let mut pitch = [0.0f32; BLOCK];
        let mut harm = [0.0f32; BLOCK];
        let mut timb = [0.0f32; BLOCK];
        let mut morph = [0.0f32; BLOCK];
        for i in 0..frames {
            pitch[i] = self.pitch.tick(ctx.smooth_k);
            harm[i] = self.harmonics.tick(ctx.smooth_k);
            timb[i] = self.timbre.tick(ctx.smooth_k);
            morph[i] = self.morph.tick(ctx.smooth_k);
        }
        let model = self.model;

        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let tcv = timbre_cv.map(|b| b.channel_or_broadcast(ch));
            let mcv = morph_cv.map(|b| b.channel_or_broadcast(ch));
            let voice = &mut self.voices[ch];
            for i in 0..frames {
                let v = pitch[i] + cv.map_or(0.0, |c| c[i]);
                let hz = voct_to_hz(v).clamp(1.0, ctx.sample_rate * 0.48);
                let dt = (hz * ctx.inv_sample_rate).min(0.49);
                let t = (timb[i] + tcv.map_or(0.0, |c| c[i] * 0.1)).clamp(0.0, 1.0);
                let m = (morph[i] + mcv.map_or(0.0, |c| c[i] * 0.1)).clamp(0.0, 1.0);
                let h = harm[i];

                let (main_s, aux_s) = match model {
                    0 => va(voice, dt, h, t, m),
                    1 => fold(voice, dt, h, t, m),
                    2 => fm(voice, hz, ctx.inv_sample_rate, h, t, m),
                    3 => additive(voice, dt, hz, ctx.sample_rate, h, t, m),
                    4 => chord(voice, v, ctx.inv_sample_rate, h, t, m),
                    _ => particle(voice, hz, t, h, m, ctx.sample_rate),
                };
                main.data[ch][i] = main_s * AUDIO_PEAK;
                aux.data[ch][i] = aux_s * AUDIO_PEAK;
            }
            voice.filt.undenorm();
        }
    }
}

impl Default for MacroOsc {
    fn default() -> Self {
        Self::new()
    }
}

// --- Models -----------------------------------------------------------------

/// Virtual analog: two detuned saws blended toward a pulse. harmonics =
/// detune, timbre = saw↔pulse blend, morph = pulse width. aux = sub square.
#[inline]
fn va(voice: &mut Voice, dt: f32, harmonics: f32, timbre: f32, morph: f32) -> (f32, f32) {
    let detune = 1.0 + harmonics * 0.04;
    let dt2 = (dt * detune).min(0.49);
    let saw1 = saw_blep(voice.phase[0], dt);
    let saw2 = saw_blep(voice.phase[1], dt2);
    let pw = 0.05 + morph * 0.9;
    let pulse = square_blep(voice.phase[0], dt, pw);
    let saws = (saw1 + saw2) * 0.5;
    let main = saws * (1.0 - timbre) + pulse * timbre;
    // Sub: square an octave down on phase[2].
    let sub = square_blep(voice.phase[2], dt * 0.5, 0.5) * 0.7;

    advance(&mut voice.phase[0], dt);
    advance(&mut voice.phase[1], dt2);
    advance(&mut voice.phase[2], dt * 0.5);
    (main, sub)
}

/// Wavefolder: a sine→triangle input driven through a sine folder. harmonics
/// = fold amount, timbre = input shape, morph = pre-fold bias. aux = pre-fold.
#[inline]
fn fold(voice: &mut Voice, dt: f32, harmonics: f32, timbre: f32, morph: f32) -> (f32, f32) {
    let s = sin01(voice.phase[0]);
    let tri = 1.0 - 4.0 * (voice.phase[0] - 0.5).abs();
    let shape = s * (1.0 - timbre) + tri * timbre;
    let bias = (morph - 0.5) * 2.0;
    let drive = 1.0 + harmonics * 6.0;
    let folded = (shape * drive + bias).sin();
    advance(&mut voice.phase[0], dt);
    (folded, shape)
}

/// 2-op FM. harmonics = ratio, timbre = index, morph = feedback. aux = mod.
#[inline]
fn fm(voice: &mut Voice, hz: f32, inv_sr: f32, harmonics: f32, timbre: f32, morph: f32) -> (f32, f32) {
    // Map harmonics to a set of musical ratios.
    let ratio = 0.5 + (harmonics * 8.0).round() * 0.5; // 0.5..4.5 in 0.5 steps
    let index = timbre * 6.0;
    let fb = morph;
    let mod_dt = (hz * ratio * inv_sr).min(0.49);
    let car_dt = (hz * inv_sr).min(0.49);
    let m = sin01(voice.mod_phase + fb * voice.last_mod * 0.5);
    voice.last_mod = m;
    let c = sin01(voice.phase[0] + index * m * 0.15);
    advance(&mut voice.mod_phase, mod_dt);
    advance(&mut voice.phase[0], car_dt);
    (c, m)
}

/// Additive harmonic stack. harmonics = partial count, timbre = rolloff,
/// morph = odd/even balance. aux = fundamental sine.
#[inline]
fn additive(voice: &mut Voice, dt: f32, f0: f32, sr: f32, harmonics: f32, timbre: f32, morph: f32) -> (f32, f32) {
    let n = (1.0 + harmonics * 15.0).round() as usize;
    let rolloff = 0.4 + timbre * 2.6;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for k in 1..=n.max(1) {
        if f0 * k as f32 > sr * 0.45 {
            break;
        }
        let mut amp = 1.0 / (k as f32).powf(rolloff);
        if k % 2 == 0 {
            amp *= morph * 2.0;
        } else {
            amp *= (1.0 - morph) * 2.0;
        }
        sum += amp * sin01(voice.phase[0] * k as f32);
        norm += amp;
    }
    let fund = sin01(voice.phase[0]);
    advance(&mut voice.phase[0], dt);
    (if norm > 0.0 { sum / norm } else { 0.0 }, fund)
}

/// Chord: stacked detuned voices. harmonics = chord type, timbre = waveform
/// (sine→saw), morph = detune spread. aux = root only.
#[inline]
fn chord(voice: &mut Voice, voct: f32, inv_sr: f32, harmonics: f32, timbre: f32, morph: f32) -> (f32, f32) {
    let chord = CHORDS[((harmonics * (CHORDS.len() as f32 - 1.0)).round() as usize).min(CHORDS.len() - 1)];
    let spread = morph * 0.03;
    let mut sum = 0.0;
    let mut root = 0.0;
    for (j, &semis) in chord.iter().enumerate().take(4) {
        let detune = if j == 0 { 0.0 } else { spread * j as f32 };
        let hz = voct_to_hz(voct + semis / 12.0 + detune);
        let dt = (hz * inv_sr).min(0.49);
        let sine = sin01(voice.phase[j]);
        let saw = saw_blep(voice.phase[j], dt);
        let s = sine * (1.0 - timbre) + saw * timbre;
        if j == 0 {
            root = s;
        }
        sum += s;
        advance(&mut voice.phase[j], dt);
    }
    (sum / chord.len() as f32, root)
}

/// Particle/noise: band-pass-filtered noise tracking pitch, blended with a
/// tone. harmonics = resonance, timbre = cutoff offset, morph = tone↔noise.
#[inline]
fn particle(voice: &mut Voice, hz: f32, timbre: f32, harmonics: f32, morph: f32, sr: f32) -> (f32, f32) {
    let cutoff = (hz * (0.5 + timbre * 3.0)).clamp(20.0, sr * 0.45);
    let q = 1.0 + harmonics * 12.0;
    let coeffs = SvfCoeffs::new(cutoff, q, sr);
    let raw = voice.noise();
    let band = voice.filt.tick(raw, &coeffs).bp;
    let tone = sin01(voice.phase[0]);
    advance(&mut voice.phase[0], (hz / sr).min(0.49));
    let main = tone * (1.0 - morph) + band * morph;
    (main, raw)
}

#[inline]
fn advance(phase: &mut f32, dt: f32) {
    *phase += dt;
    if *phase >= 1.0 {
        *phase -= 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a model for a while and return (peak, brightness) where
    /// brightness is mean abs first-difference / mean abs (a spectral proxy).
    fn analyse(osc: &mut MacroOsc) -> (f32, f64) {
        let ctx = ProcessCtx::new(48_000.0);
        let mut main = PortBuffer::silent();
        let mut aux = PortBuffer::silent();
        let mut peak = 0.0f32;
        let mut diff = 0.0f64;
        let mut tot = 0.0f64;
        let mut last = 0.0f32;
        for _ in 0..800 {
            osc.process(&ctx, None, None, None, &mut main, &mut aux, BLOCK);
            for i in 0..BLOCK {
                let s = main.data[0][i];
                assert!(s.is_finite(), "non-finite output");
                peak = peak.max(s.abs());
                diff += ((s - last) as f64).abs();
                tot += s.abs() as f64;
                last = s;
            }
        }
        (peak, diff / tot.max(1e-9))
    }

    #[test]
    fn all_models_produce_bounded_audio() {
        for model in 0..N_MODELS {
            let mut osc = MacroOsc::new();
            osc.set_param(p::MODEL, model as f32);
            osc.pitch.set_immediate(0.0);
            let (peak, _) = analyse(&mut osc);
            assert!(peak > 0.05, "model {model} silent (peak {peak})");
            assert!(peak <= AUDIO_PEAK + 0.5, "model {model} too hot (peak {peak})");
        }
    }

    #[test]
    fn va_timbre_morph_change_the_tone() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut osc = MacroOsc::new();
        osc.set_param(p::MODEL, 0.0);
        osc.pitch.set_immediate(0.0);
        // Saw vs pulse should measurably differ.
        osc.timbre.set_immediate(0.0);
        let saw = analyse(&mut osc).1;
        osc.timbre.set_immediate(1.0);
        let pulse = analyse(&mut osc).1;
        assert!((saw - pulse).abs() / saw.max(pulse) > 0.05, "VA timbre had no effect");
        let _ = ctx;
    }

    #[test]
    fn fm_index_adds_brightness() {
        let mut osc = MacroOsc::new();
        osc.set_param(p::MODEL, 2.0);
        osc.pitch.set_immediate(0.0);
        osc.harmonics.set_immediate(0.5);
        osc.timbre.set_immediate(0.0); // index 0 → pure sine
        let clean = analyse(&mut osc).1;
        osc.timbre.set_immediate(1.0); // high index
        let bright = analyse(&mut osc).1;
        assert!(bright > clean * 1.3, "FM index didn't brighten: {clean} -> {bright}");
    }

    #[test]
    fn additive_partials_increase_brightness() {
        let mut osc = MacroOsc::new();
        osc.set_param(p::MODEL, 3.0);
        osc.pitch.set_immediate(-1.0);
        osc.timbre.set_immediate(0.2); // shallow rolloff so partials show
        osc.harmonics.set_immediate(0.0); // 1 partial
        let one = analyse(&mut osc).1;
        osc.harmonics.set_immediate(1.0); // many partials
        let many = analyse(&mut osc).1;
        assert!(many > one * 1.3, "additive partials didn't brighten: {one} -> {many}");
    }

    #[test]
    fn chord_model_plays_multiple_pitches() {
        // A chord should have more zero crossings than a single oscillator at
        // the same root, since stacked higher notes cross zero more often.
        let ctx = ProcessCtx::new(48_000.0);
        let count_crossings = |osc: &mut MacroOsc| {
            let mut main = PortBuffer::silent();
            let mut aux = PortBuffer::silent();
            let mut crossings = 0u32;
            let mut last = 0.0f32;
            for _ in 0..1500 {
                osc.process(&ctx, None, None, None, &mut main, &mut aux, BLOCK);
                for i in 0..BLOCK {
                    let s = main.data[0][i];
                    if last < 0.0 && s >= 0.0 {
                        crossings += 1;
                    }
                    last = s;
                }
            }
            crossings
        };
        let mut osc = MacroOsc::new();
        osc.set_param(p::MODEL, 4.0);
        osc.pitch.set_immediate(0.0);
        osc.timbre.set_immediate(0.0); // sine voices
        // Octave chord (root + octave) vs major-7 (denser).
        osc.harmonics.set_immediate(0.0);
        let octave = count_crossings(&mut osc);
        osc.harmonics.set_immediate(1.0);
        let maj7 = count_crossings(&mut osc);
        assert!(octave > 0 && maj7 > 0);
        assert!(maj7 != octave, "chord type had no effect on the sound");
    }

    #[test]
    fn aux_output_present() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut osc = MacroOsc::new();
        osc.set_param(p::MODEL, 0.0);
        osc.pitch.set_immediate(0.0);
        let mut main = PortBuffer::silent();
        let mut aux = PortBuffer::silent();
        let mut aux_energy = 0.0f64;
        for _ in 0..200 {
            osc.process(&ctx, None, None, None, &mut main, &mut aux, BLOCK);
            for &s in &aux.data[0][..BLOCK] {
                aux_energy += (s as f64).powi(2);
            }
        }
        assert!(aux_energy > 1.0, "aux output silent");
    }
}
