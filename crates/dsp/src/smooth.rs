//! Parameter smoothing (dezippering).

/// One-pole smoother toward a target value. Branchless in the common case,
/// snaps to the target when close to avoid denormal tails.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    cur: f32,
    target: f32,
}

impl Smoothed {
    pub fn new(value: f32) -> Self {
        Self { cur: value, target: value }
    }

    /// Coefficient for a given time constant in seconds:
    /// `k = 1 - exp(-1 / (tau * sample_rate))`. Compute once at reset.
    pub fn coeff(tau_seconds: f32, sample_rate: f32) -> f32 {
        1.0 - (-1.0 / (tau_seconds * sample_rate)).exp()
    }

    #[inline]
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump immediately (used at reset / patch load).
    pub fn set_immediate(&mut self, value: f32) {
        self.cur = value;
        self.target = value;
    }

    #[inline]
    pub fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub fn current(&self) -> f32 {
        self.cur
    }

    /// Advance one sample and return the smoothed value. The snap threshold
    /// is 1e-4 because the f32 one-pole stalls once its increment drops below
    /// one ULP of the current value (~1.4e-5 short of a target near 1.0) — a
    /// tighter threshold would never trigger.
    #[inline]
    pub fn tick(&mut self, k: f32) -> f32 {
        self.cur += k * (self.target - self.cur);
        if (self.cur - self.target).abs() < 1e-4 {
            self.cur = self.target;
        }
        self.cur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_target() {
        let sr = 48_000.0;
        let k = Smoothed::coeff(0.010, sr);
        let mut s = Smoothed::new(0.0);
        s.set_target(1.0);
        // The snap threshold (1e-6) is reached after ~14 time constants;
        // 0.2 s ≈ 20 time constants pins it exactly to the target.
        for _ in 0..(sr * 0.2) as usize {
            s.tick(k);
        }
        assert_eq!(s.current(), 1.0);
    }

    #[test]
    fn ten_ms_time_constant() {
        let sr = 48_000.0;
        let k = Smoothed::coeff(0.010, sr);
        let mut s = Smoothed::new(0.0);
        s.set_target(1.0);
        for _ in 0..480 {
            s.tick(k);
        }
        // After exactly one time constant: 1 - 1/e ≈ 0.632.
        assert!((s.current() - 0.632).abs() < 0.01, "got {}", s.current());
    }
}
