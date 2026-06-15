//! Wavetable oscillator: a position knob (with CV) scans a morphing table
//! sine → triangle → saw → square. Saw/square edges use polyBLEP; the morph
//! crossfades adjacent band-limited shapes.

use crate::buffer::{PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::wtvco as p;
use rack_dsp::polyblep::{saw_blep, square_blep};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK};
use rack_dsp::Smoothed;

pub struct WtVco {
    phase: [f32; MAX_CHANNELS],
    pitch: Smoothed,
    position: Smoothed,
}

impl WtVco {
    pub fn new() -> Self {
        Self {
            phase: [0.0; MAX_CHANNELS],
            pitch: Smoothed::new(0.75),
            position: Smoothed::new(0.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::PITCH => self.pitch.set_target(value),
            p::POSITION => self.position.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        pos_cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;

        let mut pitch = [0.0f32; BLOCK];
        let mut pos = [0.0f32; BLOCK];
        for i in 0..frames {
            pitch[i] = self.pitch.tick(ctx.smooth_k);
            pos[i] = self.position.tick(ctx.smooth_k);
        }

        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let pcv = pos_cv.map(|b| b.channel_or_broadcast(ch));
            let phase = &mut self.phase[ch];
            let data = &mut out.data[ch];
            for i in 0..frames {
                let v = pitch[i] + cv.map_or(0.0, |c| c[i]);
                let dt = (voct_to_hz(v) * ctx.inv_sample_rate).min(0.45);
                let position = (pos[i] + pcv.map_or(0.0, |c| c[i]) * 0.1).clamp(0.0, 1.0);
                data[i] = morph(*phase, dt, position) * AUDIO_PEAK;
                *phase += dt;
                if *phase >= 1.0 {
                    *phase -= 1.0;
                }
            }
        }
    }
}

impl Default for WtVco {
    fn default() -> Self {
        Self::new()
    }
}

/// Crossfade across the four shapes by `position` in [0, 1].
#[inline]
fn morph(phase: f32, dt: f32, position: f32) -> f32 {
    let sine = (core::f32::consts::TAU * phase).sin();
    let tri = 1.0 - 4.0 * (phase - 0.5).abs();
    let zone = position * 3.0;
    if zone < 1.0 {
        let t = zone;
        sine * (1.0 - t) + tri * t
    } else if zone < 2.0 {
        let t = zone - 1.0;
        let saw = saw_blep(phase, dt);
        tri * (1.0 - t) + saw * t
    } else {
        let t = zone - 2.0;
        let saw = saw_blep(phase, dt);
        let sq = square_blep(phase, dt, 0.5);
        saw * (1.0 - t) + sq * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_endpoints_are_sine_and_square() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut osc = WtVco::new();
        osc.pitch.set_immediate(0.0);
        let mut out = PortBuffer::silent();

        // Position 0 ≈ sine: low harmonic content (RMS near 1/sqrt2 * peak).
        osc.position.set_immediate(0.0);
        let mut sum_sq = 0.0f64;
        let mut n = 0;
        for _ in 0..500 {
            osc.process(&ctx, None, None, &mut out, crate::buffer::BLOCK);
            for &s in &out.data[0][..crate::buffer::BLOCK] {
                assert!(s.is_finite() && s.abs() <= AUDIO_PEAK + 0.5);
                sum_sq += (s as f64).powi(2);
                n += 1;
            }
        }
        let sine_rms = (sum_sq / n as f64).sqrt();
        // Sine RMS = peak/sqrt(2) ≈ 3.54.
        assert!((sine_rms - 3.54).abs() < 0.4, "sine rms {sine_rms}");

        // Position 1 ≈ square: RMS near full peak (~5).
        osc.position.set_immediate(1.0);
        sum_sq = 0.0;
        n = 0;
        for _ in 0..500 {
            osc.process(&ctx, None, None, &mut out, crate::buffer::BLOCK);
            for &s in &out.data[0][..crate::buffer::BLOCK] {
                sum_sq += (s as f64).powi(2);
                n += 1;
            }
        }
        let sq_rms = (sum_sq / n as f64).sqrt();
        assert!(sq_rms > sine_rms + 0.8, "square rms {sq_rms} not richer than sine {sine_rms}");
    }
}
