//! Pure DSP building blocks. No engine or wasm dependencies — everything here
//! is testable natively with `cargo test`.

pub mod env;
pub mod math;
pub mod polyblep;
pub mod smooth;
pub mod svf;

pub use math::{exp2_fast, tanh_pade, undenorm};
pub use smooth::Smoothed;

/// VCV-style voltage conventions used throughout the engine.
pub mod volts {
    /// Peak amplitude of audio-rate signals (signals swing ±5 V).
    pub const AUDIO_PEAK: f32 = 5.0;
    /// Logic-high level for gates and triggers.
    pub const GATE_HIGH: f32 = 10.0;
    /// Gate detection threshold (with hysteresis around it).
    pub const GATE_THRESHOLD: f32 = 1.0;
    /// 1 V/oct reference: 0 V = middle C.
    pub const C4_HZ: f32 = 261.625_58;

    /// Convert a V/oct pitch voltage to frequency in Hz.
    #[inline]
    pub fn voct_to_hz(voct: f32) -> f32 {
        C4_HZ * super::exp2_fast(voct)
    }
}
