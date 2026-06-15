//! LFO: naive waveforms (no anti-aliasing — sub-20 Hz), bipolar ±5 V or
//! unipolar 0–10 V, with a reset (phase sync) input. Always mono.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::lfo as p;
use rack_dsp::volts::{GATE_THRESHOLD, AUDIO_PEAK};
use rack_dsp::Smoothed;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Wave {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
}

pub struct Lfo {
    phase: f32,
    rate: Smoothed,
    wave: Wave,
    bipolar: bool,
    reset_high: bool,
}

impl Lfo {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            rate: Smoothed::new(2.0),
            wave: Wave::Sine,
            bipolar: true,
            reset_high: false,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::RATE => self.rate.set_target(value.clamp(0.02, 20.0)),
            p::WAVE => {
                self.wave = match value as u32 {
                    1 => Wave::Triangle,
                    2 => Wave::Saw,
                    3 => Wave::Square,
                    _ => Wave::Sine,
                }
            }
            p::BIPOLAR => self.bipolar = value >= 0.5,
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        reset: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let reset_data = reset.map(|b| b.channel_or_broadcast(0));
        let data = &mut out.data[0];
        for i in 0..frames {
            if let Some(r) = reset_data {
                let high = r[i] >= GATE_THRESHOLD;
                if high && !self.reset_high {
                    self.phase = 0.0;
                }
                self.reset_high = high;
            }
            let rate = self.rate.tick(ctx.smooth_k);
            // Unit-amplitude bipolar shape.
            let s = match self.wave {
                Wave::Sine => (core::f32::consts::TAU * self.phase).sin(),
                Wave::Triangle => 1.0 - 4.0 * (self.phase - 0.5).abs(),
                Wave::Saw => 2.0 * self.phase - 1.0,
                Wave::Square => {
                    if self.phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            data[i] = if self.bipolar {
                s * AUDIO_PEAK
            } else {
                (s + 1.0) * AUDIO_PEAK // 0..10 V
            };
            self.phase += rate * ctx.inv_sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn rate_and_ranges() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lfo = Lfo::new();
        lfo.set_param(p::RATE, 2.0);
        let mut out = PortBuffer::silent();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        // 1 s.
        for _ in 0..1500 {
            lfo.process(&ctx, None, &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                min = min.min(s);
                max = max.max(s);
            }
        }
        assert!((max - AUDIO_PEAK).abs() < 0.1, "max {max}");
        assert!((min + AUDIO_PEAK).abs() < 0.1, "min {min}");

        // Unipolar: 0..10 V.
        lfo.set_param(p::BIPOLAR, 0.0);
        min = f32::MAX;
        max = f32::MIN;
        for _ in 0..1500 {
            lfo.process(&ctx, None, &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                min = min.min(s);
                max = max.max(s);
            }
        }
        assert!(min >= -0.01, "unipolar went negative: {min}");
        assert!((max - 10.0).abs() < 0.1, "unipolar max {max}");
    }

    #[test]
    fn reset_syncs_phase() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut lfo = Lfo::new();
        lfo.set_param(p::WAVE, 2.0); // saw: phase visible directly
        let mut out = PortBuffer::silent();
        for _ in 0..37 {
            lfo.process(&ctx, None, &mut out, BLOCK);
        }
        let mut reset = PortBuffer::silent();
        reset.data[0][0] = 10.0;
        lfo.process(&ctx, Some(&reset), &mut out, BLOCK);
        // Saw restarts from -5 V at the reset edge.
        assert!((out.data[0][0] + AUDIO_PEAK).abs() < 0.05, "got {}", out.data[0][0]);
    }
}
