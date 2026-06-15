//! VCO: polyBLEP saw/square, leaky-integrated triangle, polynomial sine.
//! Pitch is V/oct (0 V = C4); the pitch knob and the V/oct input add.

use crate::buffer::{PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::vco as p;
use rack_dsp::polyblep::{saw_blep, square_blep};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK};
use rack_dsp::Smoothed;

/// Default pitch: 0.75 V above C4 = A4 = 440 Hz.
const DEFAULT_PITCH: f32 = 0.75;
/// Triangle integrator leak (kills DC from integration).
const TRI_LEAK: f32 = 0.999;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Triangle,
    #[default]
    Saw,
    Square,
}

impl Waveform {
    fn from_f32(v: f32) -> Self {
        match v as u32 {
            0 => Self::Sine,
            1 => Self::Triangle,
            3 => Self::Square,
            _ => Self::Saw,
        }
    }
}

pub struct Vco {
    phase: [f32; MAX_CHANNELS],
    tri_state: [f32; MAX_CHANNELS],
    pitch: Smoothed,
    pulse_width: Smoothed,
    wave: Waveform,
}

impl Vco {
    pub fn new() -> Self {
        Self {
            phase: [0.0; MAX_CHANNELS],
            tri_state: [0.0; MAX_CHANNELS],
            pitch: Smoothed::new(DEFAULT_PITCH),
            pulse_width: Smoothed::new(0.5),
            wave: Waveform::default(),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::PITCH => self.pitch.set_target(value),
            p::WAVE => self.wave = Waveform::from_f32(value),
            p::PW => self.pulse_width.set_target(value.clamp(0.05, 0.95)),
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

        // Smooth knobs once per block (not per channel).
        let mut pitch_knob = [0.0f32; BLOCK];
        let mut pw = [0.5f32; BLOCK];
        for i in 0..frames {
            pitch_knob[i] = self.pitch.tick(ctx.smooth_k);
            pw[i] = self.pulse_width.tick(ctx.smooth_k);
        }

        let max_dt = 0.45; // clamp frequency to sr * 0.45
        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let phase = &mut self.phase[ch];
            let tri = &mut self.tri_state[ch];
            let data = &mut out.data[ch];
            for i in 0..frames {
                let v = pitch_knob[i] + cv.map_or(0.0, |c| c[i]);
                let dt = (voct_to_hz(v) * ctx.inv_sample_rate).min(max_dt);
                let s = match self.wave {
                    Waveform::Saw => saw_blep(*phase, dt),
                    Waveform::Square => square_blep(*phase, dt, pw[i]),
                    Waveform::Sine => sine_parabolic(*phase),
                    Waveform::Triangle => {
                        // Leaky integration of the BLEP square ≈ anti-aliased
                        // triangle; 4*dt scales the slope to unit amplitude.
                        let sq = square_blep(*phase, dt, 0.5);
                        *tri = TRI_LEAK * *tri + 4.0 * dt * sq;
                        *tri
                    }
                };
                data[i] = s * AUDIO_PEAK;
                *phase += dt;
                if *phase >= 1.0 {
                    *phase -= 1.0;
                }
            }
        }
        for tri in self.tri_state.iter_mut() {
            *tri = rack_dsp::undenorm(*tri);
        }
    }
}

impl Default for Vco {
    fn default() -> Self {
        Self::new()
    }
}

/// Parabolic sine approximation for phase in [0, 1). Error < 0.1% — fine for
/// an oscillator; avoids a libm call per sample.
#[inline]
fn sine_parabolic(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 16.0 * x * (0.5 - x.abs());
    0.225 * (y * y.abs() - y) + y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_approx_is_close() {
        for i in 0..1000 {
            let phase = i as f32 / 1000.0;
            let approx = sine_parabolic(phase);
            let exact = (std::f32::consts::TAU * phase).sin();
            assert!((approx - exact).abs() < 0.002, "phase {phase}: {approx} vs {exact}");
        }
    }

    #[test]
    fn poly_voct_input_sets_channel_count_and_pitch() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut vco = Vco::new();
        vco.pitch.set_immediate(0.0); // C4 on the knob

        let mut voct = PortBuffer::silent();
        voct.channels = 2;
        // Channel 0 at knob pitch, channel 1 an octave up.
        voct.data[1] = [1.0; BLOCK];

        let mut out = PortBuffer::silent();
        // Render ~1s, count rising zero crossings per channel.
        let mut crossings = [0u32; 2];
        let mut last = [0.0f32; 2];
        for _ in 0..1500 {
            vco.process(&ctx, Some(&voct), &mut out, BLOCK);
            assert_eq!(out.channels, 2);
            for ch in 0..2 {
                for &s in out.data[ch].iter() {
                    if last[ch] < 0.0 && s >= 0.0 {
                        crossings[ch] += 1;
                    }
                    last[ch] = s;
                }
            }
        }
        let ratio = crossings[1] as f32 / crossings[0] as f32;
        assert!((ratio - 2.0).abs() < 0.05, "octave ratio {ratio} ({crossings:?})");
    }
}
