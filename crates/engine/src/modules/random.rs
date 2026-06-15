//! Random / chaos source: on each trigger (external, or an internal clock at
//! `rate` Hz when nothing is patched), latch a new random voltage. Two
//! outputs — a stepped sample & hold and a slew-smoothed version — for both
//! burbling stepped melodies and smooth drifting modulation.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::random as p;
use rack_dsp::volts::{AUDIO_PEAK, GATE_THRESHOLD};
use rack_dsp::Smoothed;

pub struct Random {
    rng: u32,
    stepped: f32,
    smooth: f32,
    phase: f32,
    rate: Smoothed,
    slew: Smoothed,
    trig_high: bool,
}

impl Random {
    pub fn new() -> Self {
        Self {
            rng: 0x9e3779b9,
            stepped: 0.0,
            smooth: 0.0,
            phase: 0.0,
            rate: Smoothed::new(4.0),
            slew: Smoothed::new(0.2),
            trig_high: false,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::RATE => self.rate.set_target(value.clamp(0.1, 30.0)),
            p::SLEW => self.slew.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn next_random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        ((x as f32 / u32::MAX as f32) * 2.0 - 1.0) * AUDIO_PEAK
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        trig: Option<&PortBuffer>,
        stepped_out: &mut PortBuffer,
        smooth_out: &mut PortBuffer,
        frames: usize,
    ) {
        stepped_out.channels = 1;
        smooth_out.channels = 1;
        let trig_data = trig.map(|b| b.channel_or_broadcast(0));

        for i in 0..frames {
            let rate = self.rate.tick(ctx.smooth_k);
            let slew = self.slew.tick(ctx.smooth_k);

            let fire = if let Some(t) = trig_data {
                let high = t[i] >= GATE_THRESHOLD;
                let edge = high && !self.trig_high;
                self.trig_high = high;
                edge
            } else {
                // Internal clock.
                self.phase += rate * ctx.inv_sample_rate;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                    true
                } else {
                    false
                }
            };
            if fire {
                self.stepped = self.next_random();
            }

            // Smooth output: one-pole toward the stepped value, slower as the
            // slew knob rises (up to ~0.3 s time constant).
            let k = if slew <= 0.0 {
                1.0
            } else {
                Smoothed::coeff(slew * 0.3, ctx.sample_rate)
            };
            self.smooth += k * (self.stepped - self.smooth);

            stepped_out.data[0][i] = self.stepped;
            smooth_out.data[0][i] = self.smooth;
        }
    }
}

impl Default for Random {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn external_trigger_latches_new_values() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut r = Random::new();
        r.slew.set_immediate(0.0); // smooth tracks instantly for the check
        let mut step = PortBuffer::silent();
        let mut smooth = PortBuffer::silent();
        let mut trig = PortBuffer::silent();

        // No trigger yet: stepped holds its initial 0.
        r.process(&ctx, Some(&trig), &mut step, &mut smooth, BLOCK);
        assert_eq!(step.data[0][BLOCK - 1], 0.0);

        // Trig goes high (rising edge at sample 0): a new in-range value latches.
        trig.data[0] = [10.0; BLOCK];
        r.process(&ctx, Some(&trig), &mut step, &mut smooth, BLOCK);
        let v = step.data[0][BLOCK - 1];
        assert!(v != 0.0 && v.abs() <= AUDIO_PEAK, "latched {v}");

        // Trig stays high (no new rising edge): value holds.
        let held = v;
        r.process(&ctx, Some(&trig), &mut step, &mut smooth, BLOCK);
        assert_eq!(step.data[0][BLOCK - 1], held);
    }

    #[test]
    fn internal_clock_produces_changing_values() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut r = Random::new();
        r.rate.set_immediate(20.0);
        let mut step = PortBuffer::silent();
        let mut smooth = PortBuffer::silent();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            r.process(&ctx, None, &mut step, &mut smooth, BLOCK);
            seen.insert(step.data[0][BLOCK - 1].to_bits());
            for &s in &step.data[0][..BLOCK] {
                assert!(s.abs() <= AUDIO_PEAK);
            }
        }
        assert!(seen.len() > 10, "internal clock not stepping: {} distinct", seen.len());
    }

    #[test]
    fn smooth_output_is_less_jumpy_than_stepped() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut r = Random::new();
        r.rate.set_immediate(15.0);
        r.slew.set_immediate(0.8);
        let mut step = PortBuffer::silent();
        let mut smooth = PortBuffer::silent();
        let mut step_jump = 0.0f64;
        let mut smooth_jump = 0.0f64;
        let (mut ls, mut lm) = (0.0f32, 0.0f32);
        for _ in 0..3000 {
            r.process(&ctx, None, &mut step, &mut smooth, BLOCK);
            for i in 0..BLOCK {
                step_jump += ((step.data[0][i] - ls) as f64).abs();
                smooth_jump += ((smooth.data[0][i] - lm) as f64).abs();
                ls = step.data[0][i];
                lm = smooth.data[0][i];
            }
        }
        assert!(smooth_jump < step_jump * 0.5, "smooth {smooth_jump} not smoother than stepped {step_jump}");
    }
}
