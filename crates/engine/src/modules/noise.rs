//! Noise source: white (xorshift) or pink (Paul Kellet's economy filter).

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::noise as p;
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::Smoothed;

pub struct Noise {
    rng: u32,
    pink: [f32; 3],
    white_kind: bool,
    level: Smoothed,
}

impl Noise {
    pub fn new() -> Self {
        Self { rng: 0x12345678, pink: [0.0; 3], white_kind: true, level: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::KIND => self.white_kind = (value as u32) == 0,
            p::LEVEL => self.level.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn white(&mut self) -> f32 {
        // xorshift32 → [-1, 1].
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn process(&mut self, ctx: &ProcessCtx, out: &mut PortBuffer, frames: usize) {
        out.channels = 1;
        for i in 0..frames {
            let level = self.level.tick(ctx.smooth_k);
            let w = self.white();
            let s = if self.white_kind {
                w
            } else {
                // Paul Kellet's 3-pole pink approximation (±~0.5 dB/oct err).
                self.pink[0] = 0.99765 * self.pink[0] + w * 0.0990460;
                self.pink[1] = 0.96300 * self.pink[1] + w * 0.2965164;
                self.pink[2] = 0.57000 * self.pink[2] + w * 1.0526913;
                (self.pink[0] + self.pink[1] + self.pink[2] + w * 0.1848) * 0.2
            };
            out.data[0][i] = s * AUDIO_PEAK * level;
        }
    }
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn white_noise_is_loud_zero_mean_and_bounded() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut noise = Noise::new();
        let mut out = PortBuffer::silent();
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        let mut n = 0u32;
        for _ in 0..3000 {
            noise.process(&ctx, &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                assert!(s.abs() <= AUDIO_PEAK + 0.01);
                sum += s as f64;
                sum_sq += (s * s) as f64;
                n += 1;
            }
        }
        let mean = sum / n as f64;
        let rms = (sum_sq / n as f64).sqrt();
        assert!(mean.abs() < 0.05, "mean {mean}");
        // Uniform white over ±5: rms = 5/sqrt(3) ≈ 2.89.
        assert!((rms - 2.89).abs() < 0.15, "rms {rms}");
    }

    #[test]
    fn pink_noise_has_less_high_frequency_energy() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut noise = Noise::new();
        noise.set_param(p::KIND, 1.0);
        let mut out = PortBuffer::silent();
        // Crude spectral tilt check: first-difference energy / signal energy
        // is much lower for pink than white.
        let tilt = |noise: &mut Noise, out: &mut PortBuffer| {
            let mut diff = 0.0f64;
            let mut total = 0.0f64;
            let mut last = 0.0f32;
            for _ in 0..2000 {
                noise.process(&ctx, out, BLOCK);
                for &s in &out.data[0][..BLOCK] {
                    diff += ((s - last) as f64).powi(2);
                    total += (s as f64).powi(2);
                    last = s;
                }
            }
            diff / total
        };
        let pink_tilt = tilt(&mut noise, &mut out);
        noise.set_param(p::KIND, 0.0);
        let white_tilt = tilt(&mut noise, &mut out);
        assert!(
            pink_tilt < white_tilt * 0.4,
            "pink {pink_tilt} not darker than white {white_tilt}"
        );
    }
}
