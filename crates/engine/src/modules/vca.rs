//! VCA: gain knob × CV (0–10 V normalized, normalled fully open). The
//! exponential response squares the normalized CV — cheap and close enough.

use crate::buffer::{propagate_channels, PortBuffer};
use crate::ProcessCtx;
use rack_core::modules::params::vca as p;
use rack_dsp::volts::GATE_HIGH;
use rack_dsp::Smoothed;

pub struct Vca {
    gain: Smoothed,
    exponential: bool,
}

impl Vca {
    pub fn new() -> Self {
        Self { gain: Smoothed::new(1.0), exponential: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::GAIN => self.gain.set_target(value.clamp(0.0, 1.0)),
            p::RESPONSE => self.exponential = value >= 0.5,
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, cv]);
        out.channels = channels;

        // Smooth the gain knob once per frame (not per channel × frame).
        let mut gain = [0.0f32; crate::buffer::BLOCK];
        for g in gain.iter_mut().take(frames) {
            *g = self.gain.tick(ctx.smooth_k);
        }

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let cv_data = cv.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                let mut g = cv_data.map_or(1.0, |c| (c[i] / GATE_HIGH).clamp(0.0, 1.0));
                if self.exponential {
                    g *= g;
                }
                data[i] = x * gain[i] * g;
            }
        }
    }
}

impl Default for Vca {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn cv_scales_and_normals_open() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut vca = Vca::new();

        let mut input = PortBuffer::silent();
        input.data[0] = [2.0; BLOCK];
        let mut cv = PortBuffer::silent();
        cv.data[0] = [5.0; BLOCK]; // half open
        let mut out = PortBuffer::silent();

        // No CV connected: fully open.
        vca.process(&ctx, Some(&input), None, &mut out, BLOCK);
        assert!((out.data[0][BLOCK - 1] - 2.0).abs() < 1e-5);

        // CV at 5 V: half gain (linear response).
        vca.process(&ctx, Some(&input), Some(&cv), &mut out, BLOCK);
        assert!((out.data[0][BLOCK - 1] - 1.0).abs() < 1e-5);

        // Exponential response: (0.5)^2 = 0.25.
        vca.set_param(p::RESPONSE, 1.0);
        vca.process(&ctx, Some(&input), Some(&cv), &mut out, BLOCK);
        assert!((out.data[0][BLOCK - 1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn poly_input_with_mono_cv_broadcasts() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut vca = Vca::new();
        let mut input = PortBuffer::silent();
        input.channels = 4;
        for ch in 0..4 {
            input.data[ch] = [(ch + 1) as f32; BLOCK];
        }
        let mut cv = PortBuffer::silent();
        cv.data[0] = [5.0; BLOCK];
        let mut out = PortBuffer::silent();
        vca.process(&ctx, Some(&input), Some(&cv), &mut out, BLOCK);
        assert_eq!(out.channels, 4);
        for ch in 0..4 {
            let expect = (ch + 1) as f32 * 0.5;
            assert!((out.data[ch][BLOCK - 1] - expect).abs() < 1e-5);
        }
    }
}
