//! Fast math approximations. WASM has no FTZ/DAZ and libm calls are slow in
//! per-sample paths, so pitch conversion and denormal handling live here.

/// Fast 2^x for x in roughly [-60, 60]. Splits into a rounded integer
/// exponent (assembled directly into the float's exponent bits) and a
/// degree-6 polynomial on [-0.5, 0.5] (Cephes exp2f coefficients).
/// Relative error < 4e-7 — far below audible pitch error (1 cent ≈ 6e-4).
#[inline]
pub fn exp2_fast(x: f32) -> f32 {
    let xi = x.round();
    let xf = x - xi; // in [-0.5, 0.5]
    let p = 1.0
        + xf * (0.693_147_2
            + xf * (0.240_226_48
                + xf * (0.055_503_327
                    + xf * (0.009_618_437
                        + xf * (0.001_339_887_4 + xf * 0.000_153_533_62)))));
    let bits = (((xi as i32) + 127) << 23) as u32;
    f32::from_bits(bits) * p
}

/// Padé tanh approximation: transparent below |x| ≈ 1, saturates smoothly,
/// hard-bounded at ±1. No libm call; used for soft clipping and filter drive.
#[inline]
pub fn tanh_pade(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    // The final clamp kills the one-ULP overshoot at |x| = 3.
    (x * (27.0 + x * x) / (27.0 + 9.0 * x * x)).clamp(-1.0, 1.0)
}

/// Flush tiny values to zero. WASM cannot set flush-to-zero mode, so
/// recursive structures (filter states, envelope tails, smoothers) call this
/// once per block to keep denormals from stalling the host CPU.
#[inline]
pub fn undenorm(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp2_fast_accuracy() {
        // Sweep the musically relevant range at fine resolution.
        let mut v = -10.0f32;
        while v <= 10.0 {
            let approx = exp2_fast(v);
            let exact = v.exp2();
            let rel = ((approx - exact) / exact).abs();
            assert!(rel < 2e-6, "exp2_fast({v}) = {approx}, exact {exact}, rel err {rel}");
            v += 0.001;
        }
    }

    #[test]
    fn exp2_fast_voct_pitch() {
        // A4 = C4 * 2^0.75 must be 440 Hz within a tiny fraction of a cent.
        let a4 = crate::volts::voct_to_hz(0.75);
        assert!((a4 - 440.0).abs() < 0.01, "A4 came out as {a4}");
    }

    #[test]
    fn undenorm_flushes() {
        assert_eq!(undenorm(1e-30), 0.0);
        assert_eq!(undenorm(-1e-30), 0.0);
        assert_eq!(undenorm(1e-10), 1e-10);
        assert_eq!(undenorm(0.5), 0.5);
    }
}
