//! Mixer: four inputs with level knobs into a master level. Plain sum, no
//! normalization; polyphony passes through (max of input channel counts).

use crate::buffer::{propagate_channels, PortBuffer, BLOCK};
use crate::ProcessCtx;
use rack_core::modules::params::mixer as p;
use rack_dsp::Smoothed;

pub struct Mixer {
    levels: [Smoothed; 4],
    master: Smoothed,
}

impl Mixer {
    pub fn new() -> Self {
        Self { levels: [Smoothed::new(0.8); 4], master: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        let v = value.clamp(0.0, 1.0);
        match param {
            p::LEVEL1 | p::LEVEL2 | p::LEVEL3 | p::LEVEL4 => {
                self.levels[param as usize].set_target(v)
            }
            p::MASTER => self.master.set_target(v),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        inputs: [Option<&PortBuffer>; 4],
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&inputs);
        out.channels = channels;

        // Smooth all gains once per frame.
        let mut gains = [[0.0f32; BLOCK]; 4];
        let mut master = [0.0f32; BLOCK];
        for i in 0..frames {
            for (g, l) in gains.iter_mut().zip(self.levels.iter_mut()) {
                g[i] = l.tick(ctx.smooth_k);
            }
            master[i] = self.master.tick(ctx.smooth_k);
        }

        for ch in 0..channels as usize {
            let data = &mut out.data[ch];
            data[..frames].fill(0.0);
            for (input, gain) in inputs.iter().zip(gains.iter()) {
                if let Some(b) = input {
                    let src = b.channel_or_broadcast(ch);
                    for i in 0..frames {
                        data[i] += src[i] * gain[i];
                    }
                }
            }
            for i in 0..frames {
                data[i] *= master[i];
            }
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_with_levels() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut mixer = Mixer::new();
        mixer.levels[0].set_immediate(1.0);
        mixer.levels[1].set_immediate(0.5);
        mixer.master.set_immediate(1.0);

        let mut a = PortBuffer::silent();
        a.data[0] = [2.0; BLOCK];
        let mut b = PortBuffer::silent();
        b.data[0] = [4.0; BLOCK];

        let mut out = PortBuffer::silent();
        mixer.process(&ctx, [Some(&a), Some(&b), None, None], &mut out, BLOCK);
        // 2*1 + 4*0.5 = 4.
        assert!((out.data[0][BLOCK - 1] - 4.0).abs() < 1e-4, "got {}", out.data[0][BLOCK - 1]);
    }
}
