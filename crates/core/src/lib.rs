//! Shared vocabulary between the UI app and the audio worklet: compile-time
//! caps, POD message types, and (behind the `patch` feature) the serde patch
//! model. No wasm dependencies — both sides and native tests use this crate.

pub mod messages;
pub mod meters;
pub mod modules;
pub mod plan;

/// Compile-time engine caps. All audio-side pools are sized by these.
pub mod caps {
    /// Frames per processing sub-block (4 per 128-frame Web Audio quantum).
    pub const BLOCK: usize = 32;
    /// Polyphonic cable width, VCV-style.
    pub const MAX_CHANNELS: usize = 16;
    /// Module slot pool size.
    pub const MAX_MODULES: usize = 256;
    /// Cable pool size.
    pub const MAX_CABLES: usize = 1024;
}
