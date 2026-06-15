//! ADSR envelope: explicit state machine with exponential segments via
//! one-pole toward an overshoot target (attack aims past 1.0 so the segment
//! terminates; decay/release converge asymptotically like analog envelopes).

use crate::math::undenorm;

const ATTACK_OVERSHOOT: f32 = 1.3;
const IDLE_FLOOR: f32 = 1e-4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Per-segment one-pole coefficients, recomputed when knobs change.
#[derive(Clone, Copy, Debug)]
pub struct AdsrParams {
    pub attack_k: f32,
    pub decay_k: f32,
    pub release_k: f32,
    pub sustain: f32,
}

impl AdsrParams {
    /// Times in seconds; coefficient maps "time" to one time constant.
    pub fn new(attack_s: f32, decay_s: f32, sustain: f32, release_s: f32, sample_rate: f32) -> Self {
        let k = |t: f32| 1.0 - (-1.0 / (t.max(0.0005) * sample_rate)).exp();
        Self {
            attack_k: k(attack_s),
            decay_k: k(decay_s),
            release_k: k(release_s),
            sustain: sustain.clamp(0.0, 1.0),
        }
    }
}

/// One envelope lane (one per poly channel).
#[derive(Clone, Copy, Debug, Default)]
pub struct Adsr {
    stage: EnvStage,
    level: f32,
}

impl Adsr {
    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
    }

    pub fn stage(&self) -> EnvStage {
        self.stage
    }

    /// Gate rising edge: restart attack from the current level (no click).
    pub fn gate_on(&mut self) {
        self.stage = EnvStage::Attack;
    }

    /// Gate falling edge.
    pub fn gate_off(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    /// Advance one sample, returning the envelope level in [0, 1].
    #[inline]
    pub fn tick(&mut self, p: &AdsrParams) -> f32 {
        match self.stage {
            EnvStage::Idle => {}
            EnvStage::Attack => {
                self.level += p.attack_k * (ATTACK_OVERSHOOT - self.level);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                self.level += p.decay_k * (p.sustain - self.level);
                if (self.level - p.sustain).abs() < 1e-4 {
                    self.stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => {
                // Track sustain knob movement while held.
                self.level = p.sustain;
            }
            EnvStage::Release => {
                self.level += p.release_k * (0.0 - self.level);
                if self.level < IDLE_FLOOR {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }
        self.level
    }

    /// Call once per block to keep the level out of denormal range.
    #[inline]
    pub fn undenorm(&mut self) {
        self.level = undenorm(self.level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn full_cycle() {
        let p = AdsrParams::new(0.01, 0.05, 0.5, 0.1, SR);
        let mut env = Adsr::default();

        env.gate_on();
        // Attack reaches 1.0 then enters decay.
        let mut hit_peak = false;
        for _ in 0..(SR * 0.1) as usize {
            if env.tick(&p) >= 1.0 {
                hit_peak = true;
                break;
            }
        }
        assert!(hit_peak, "attack never reached peak");
        assert_eq!(env.stage(), EnvStage::Decay);

        // Decay settles to sustain.
        for _ in 0..(SR * 0.5) as usize {
            env.tick(&p);
        }
        assert_eq!(env.stage(), EnvStage::Sustain);
        assert!((env.tick(&p) - 0.5).abs() < 0.01);

        // Release decays to idle.
        env.gate_off();
        for _ in 0..(SR * 2.0) as usize {
            env.tick(&p);
        }
        assert_eq!(env.stage(), EnvStage::Idle);
        assert_eq!(env.tick(&p), 0.0);
    }

    #[test]
    fn attack_time_is_roughly_right() {
        // With the 1.3 overshoot target, level crosses 1.0 in about
        // -ln(1 - 1/1.3) ≈ 1.47 time constants.
        let attack_s = 0.1;
        let p = AdsrParams::new(attack_s, 0.05, 0.5, 0.1, SR);
        let mut env = Adsr::default();
        env.gate_on();
        let mut samples = 0u32;
        while env.stage() == EnvStage::Attack {
            env.tick(&p);
            samples += 1;
            assert!(samples < SR as u32, "attack never terminated");
        }
        let t = samples as f32 / SR;
        assert!((t - 0.147).abs() < 0.02, "attack took {t}s");
    }

    #[test]
    fn retrigger_from_release_does_not_jump() {
        let p = AdsrParams::new(0.01, 0.05, 0.8, 0.5, SR);
        let mut env = Adsr::default();
        env.gate_on();
        for _ in 0..(SR * 0.2) as usize {
            env.tick(&p);
        }
        env.gate_off();
        for _ in 0..100 {
            env.tick(&p);
        }
        let before = env.tick(&p);
        env.gate_on();
        let after = env.tick(&p);
        // Restarting attack continues from the current level — small step only.
        assert!((after - before).abs() < 0.01, "click on retrigger: {before} -> {after}");
    }

    #[test]
    fn gate_off_while_idle_stays_idle() {
        let p = AdsrParams::new(0.01, 0.05, 0.5, 0.1, SR);
        let mut env = Adsr::default();
        env.gate_off();
        assert_eq!(env.stage(), EnvStage::Idle);
        assert_eq!(env.tick(&p), 0.0);
    }
}
