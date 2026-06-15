//! State-variable filter using Andrew Simper's trapezoidal integration
//! (cytomic "SvfLinearTrapOptimised2"). Unconditionally stable under
//! audio-rate cutoff modulation; LP/BP/HP fall out of the same tick.

use crate::math::undenorm;

/// Coefficients, recomputed per block (one `tan` per block, not per sample).
#[derive(Clone, Copy, Debug)]
pub struct SvfCoeffs {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl SvfCoeffs {
    pub fn new(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let cutoff = cutoff_hz.clamp(10.0, sample_rate * 0.45);
        let g = (core::f32::consts::PI * cutoff / sample_rate).tan();
        let k = 1.0 / q.max(0.5);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { a1, a2, a3, k }
    }
}

/// Per-channel filter state (two integrator states).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1eq: f32,
    ic2eq: f32,
}

/// One tick's outputs.
pub struct SvfOut {
    pub lp: f32,
    pub bp: f32,
    pub hp: f32,
}

impl Svf {
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Call once per block to keep filter states out of denormal range.
    #[inline]
    pub fn undenorm(&mut self) {
        self.ic1eq = undenorm(self.ic1eq);
        self.ic2eq = undenorm(self.ic2eq);
    }

    #[inline]
    pub fn tick(&mut self, v0: f32, c: &SvfCoeffs) -> SvfOut {
        let v3 = v0 - self.ic2eq;
        let v1 = c.a1 * self.ic1eq + c.a2 * v3;
        let v2 = self.ic2eq + c.a2 * self.ic1eq + c.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        SvfOut {
            lp: v2,
            bp: v1,
            hp: v0 - c.k * v1 - v2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measure steady-state gain of a sine through the filter via RMS
    /// (sample peaks under-read at high frequencies — few samples/period).
    fn measure_gain(cutoff: f32, q: f32, freq: f32, sr: f32, pick: fn(&SvfOut) -> f32) -> f32 {
        let c = SvfCoeffs::new(cutoff, q, sr);
        let mut f = Svf::default();
        let n = (sr as usize) / 2;
        let mut sum_sq = 0.0f64;
        let mut count = 0u32;
        for i in 0..n {
            let x = (std::f32::consts::TAU * freq * i as f32 / sr).sin();
            let out = pick(&f.tick(x, &c));
            // Skip the transient before measuring.
            if i > n / 2 {
                sum_sq += (out as f64) * (out as f64);
                count += 1;
            }
        }
        // RMS of unit-amplitude sine is 1/sqrt(2); scale back to amplitude.
        ((sum_sq / count as f64).sqrt() * std::f64::consts::SQRT_2) as f32
    }

    #[test]
    fn lowpass_passes_low_attenuates_high() {
        let sr = 48_000.0;
        let low = measure_gain(1_000.0, 0.7071, 100.0, sr, |o| o.lp);
        let high = measure_gain(1_000.0, 0.7071, 8_000.0, sr, |o| o.lp);
        assert!((low - 1.0).abs() < 0.05, "passband gain {low}");
        // 3 octaves above cutoff, 2-pole: ~ -36 dB → amplitude < 0.03.
        assert!(high < 0.03, "stopband gain {high}");
    }

    #[test]
    fn cutoff_gain_matches_butterworth() {
        // At cutoff with q = 1/sqrt(2), |H| should be 1/sqrt(2) ≈ 0.707.
        let g = measure_gain(1_000.0, std::f32::consts::FRAC_1_SQRT_2, 1_000.0, 48_000.0, |o| o.lp);
        assert!((g - 0.707).abs() < 0.03, "cutoff gain {g}");
    }

    #[test]
    fn highpass_mirrors_lowpass() {
        let sr = 48_000.0;
        let low = measure_gain(1_000.0, 0.7071, 100.0, sr, |o| o.hp);
        let high = measure_gain(1_000.0, 0.7071, 8_000.0, sr, |o| o.hp);
        assert!(low < 0.05, "hp leaked low freq: {low}");
        assert!((high - 1.0).abs() < 0.05, "hp passband gain {high}");
    }

    #[test]
    fn stable_under_audio_rate_modulation() {
        // Sweep cutoff wildly every sample; output must stay bounded.
        let sr = 48_000.0;
        let mut f = Svf::default();
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let t = i as f32 / sr;
            let cutoff = 200.0 + 10_000.0 * (0.5 + 0.5 * (std::f32::consts::TAU * 3_000.0 * t).sin());
            let c = SvfCoeffs::new(cutoff, 8.0, sr);
            let x = (std::f32::consts::TAU * 220.0 * t).sin();
            let out = f.tick(x, &c);
            peak = peak.max(out.lp.abs());
            assert!(out.lp.is_finite());
        }
        assert!(peak < 20.0, "filter blew up: peak {peak}");
    }
}
