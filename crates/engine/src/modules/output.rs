//! Output module: sums poly channels, scales ±5 V to ±1.0, soft-clips, and
//! writes the final stereo stream. Right input is normalled to left.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_dsp::tanh_pade;
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::Smoothed;

use rack_core::modules::params::output as p;

pub struct OutputModule {
    level: Smoothed,
}

impl OutputModule {
    pub fn new() -> Self {
        Self { level: Smoothed::new(0.8) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == p::LEVEL {
            self.level.set_target(value.clamp(0.0, 1.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        left_in: Option<&PortBuffer>,
        right_in: Option<&PortBuffer>,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        let frames = out_l.len().min(out_r.len());
        let right_in = right_in.or(left_in); // R normalled to L

        for i in 0..frames {
            let gain = self.level.tick(ctx.smooth_k) / AUDIO_PEAK;
            out_l[i] = soft_clip(sum_channels(left_in, i) * gain);
            out_r[i] = soft_clip(sum_channels(right_in, i) * gain);
        }
    }
}

impl Default for OutputModule {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn sum_channels(buf: Option<&PortBuffer>, frame: usize) -> f32 {
    match buf {
        Some(b) => {
            let mut sum = 0.0;
            for ch in 0..b.channels.max(1) as usize {
                sum += b.data[ch][frame];
            }
            sum
        }
        None => 0.0,
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    tanh_pade(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn soft_clip_is_bounded_and_transparent() {
        for i in -1000..=1000 {
            let x = i as f32 / 100.0;
            let y = soft_clip(x);
            assert!(y.abs() <= 1.0, "clip({x}) = {y}");
        }
        // Near-linear for small signals.
        assert!((soft_clip(0.1) - 0.1).abs() < 0.001);
    }

    #[test]
    fn sums_poly_channels_and_normals_right_to_left() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut out_mod = OutputModule::new();
        out_mod.level.set_immediate(1.0);

        let mut input = PortBuffer::silent();
        input.channels = 4;
        for ch in 0..4 {
            input.data[ch] = [0.5; BLOCK]; // sums to 2.0 V
        }

        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        out_mod.process(&ctx, Some(&input), None, &mut l, &mut r);
        // 2.0 V / 5 V = 0.4 through the tanh-shaped clipper ≈ tanh(0.4).
        assert!((l[0] - 0.4f32.tanh()).abs() < 0.005, "got {}", l[0]);
        assert_eq!(l, r);
    }
}
