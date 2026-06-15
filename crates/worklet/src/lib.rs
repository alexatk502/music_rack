//! wasm-bindgen wrapper around the engine, instantiated inside the
//! AudioWorkletGlobalScope by `web/worklet-shim.js`.

use rack_engine::Engine;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WorkletEngine {
    engine: Engine,
}

#[wasm_bindgen]
impl WorkletEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: f32) -> WorkletEngine {
        WorkletEngine { engine: Engine::new(sample_rate) }
    }

    /// Render one quantum. wasm-bindgen copies the JS Float32Arrays in and
    /// back out — unavoidable since the worklet owns those buffers, and cheap
    /// at 128 frames.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.engine.process(left, right);
    }

    /// Apply a batch of 32-byte control records (see rack-core::messages).
    pub fn on_message(&mut self, bytes: &[u8]) {
        self.engine.on_message(bytes);
    }

    /// Meter snapshot blob for the UI. The shim calls this every ~30 ms and
    /// posts the bytes back through the port.
    pub fn take_meters(&mut self) -> Vec<u8> {
        self.engine.take_meters()
    }
}
