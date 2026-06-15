//! ADSR module: per-channel envelope lanes driven by gate (threshold with
//! hysteresis) and retrigger inputs. Output is unipolar 0–10 V.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::adsr as p;
use rack_dsp::env::{Adsr, AdsrParams};
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};

pub struct AdsrModule {
    env: [Adsr; MAX_CHANNELS],
    gate_high: [bool; MAX_CHANNELS],
    retrig_high: [bool; MAX_CHANNELS],
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
}

impl AdsrModule {
    pub fn new() -> Self {
        Self {
            env: [Adsr::default(); MAX_CHANNELS],
            gate_high: [false; MAX_CHANNELS],
            retrig_high: [false; MAX_CHANNELS],
            attack_s: 0.01,
            decay_s: 0.2,
            sustain: 0.7,
            release_s: 0.3,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::ATTACK => self.attack_s = value.max(0.0005),
            p::DECAY => self.decay_s = value.max(0.0005),
            p::SUSTAIN => self.sustain = value.clamp(0.0, 1.0),
            p::RELEASE => self.release_s = value.max(0.0005),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        gate: Option<&PortBuffer>,
        retrig: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[gate, retrig]);
        out.channels = channels;
        // Coefficients once per block (3 exps; knob smoothing is overkill
        // for segment times).
        let params =
            AdsrParams::new(self.attack_s, self.decay_s, self.sustain, self.release_s, ctx.sample_rate);

        for ch in 0..channels as usize {
            let gate_data = gate.map(|b| b.channel_or_broadcast(ch));
            let retrig_data = retrig.map(|b| b.channel_or_broadcast(ch));
            let env = &mut self.env[ch];
            let data = &mut out.data[ch];
            for i in 0..frames {
                // Gate edge detection with hysteresis (1 V on, 0.5 V off).
                if let Some(g) = gate_data {
                    let high = if self.gate_high[ch] {
                        g[i] > GATE_THRESHOLD * 0.5
                    } else {
                        g[i] >= GATE_THRESHOLD
                    };
                    if high != self.gate_high[ch] {
                        self.gate_high[ch] = high;
                        if high {
                            env.gate_on();
                        } else {
                            env.gate_off();
                        }
                    }
                }
                if let Some(r) = retrig_data {
                    let high = r[i] >= GATE_THRESHOLD;
                    if high && !self.retrig_high[ch] && self.gate_high[ch] {
                        env.gate_on(); // restart attack from current level
                    }
                    self.retrig_high[ch] = high;
                }
                data[i] = env.tick(&params) * GATE_HIGH;
            }
            env.undenorm();
        }
    }
}

impl Default for AdsrModule {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn gate_drives_envelope() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut adsr = AdsrModule::new();
        adsr.set_param(p::ATTACK, 0.002);
        adsr.set_param(p::RELEASE, 0.05);

        let mut gate = PortBuffer::silent();
        gate.data[0] = [GATE_HIGH; BLOCK];
        let mut out = PortBuffer::silent();

        // Gate high: envelope rises.
        for _ in 0..100 {
            adsr.process(&ctx, Some(&gate), None, &mut out, BLOCK);
        }
        assert!(out.data[0][BLOCK - 1] > 3.0, "envelope flat: {}", out.data[0][BLOCK - 1]);

        // Gate low: envelope releases to zero.
        gate.data[0] = [0.0; BLOCK];
        for _ in 0..3000 {
            adsr.process(&ctx, Some(&gate), None, &mut out, BLOCK);
        }
        assert_eq!(out.data[0][BLOCK - 1], 0.0);
    }

    #[test]
    fn retrig_restarts_attack_while_held() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut adsr = AdsrModule::new();
        adsr.set_param(p::SUSTAIN, 0.4);
        adsr.set_param(p::DECAY, 0.01);

        let mut gate = PortBuffer::silent();
        gate.data[0] = [GATE_HIGH; BLOCK];
        let mut out = PortBuffer::silent();
        // Settle into sustain.
        for _ in 0..2000 {
            adsr.process(&ctx, Some(&gate), None, &mut out, BLOCK);
        }
        let sustain_level = out.data[0][BLOCK - 1];
        assert!((sustain_level - 4.0).abs() < 0.2, "sustain at {sustain_level}");

        // Retrig pulse → re-attack above sustain.
        let mut retrig = PortBuffer::silent();
        retrig.data[0] = [GATE_HIGH; BLOCK];
        adsr.process(&ctx, Some(&gate), Some(&retrig), &mut out, BLOCK);
        let mut peak = 0.0f32;
        let zero = PortBuffer::silent();
        for _ in 0..200 {
            adsr.process(&ctx, Some(&gate), Some(&zero), &mut out, BLOCK);
            peak = peak.max(out.data[0][BLOCK - 1]);
        }
        assert!(peak > sustain_level + 2.0, "no re-attack: peak {peak}");
    }
}
