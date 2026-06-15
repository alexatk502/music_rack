//! Main-thread side of the control path: UI mutations queue [`Msg`] records,
//! and the queue is flushed as one transferable ArrayBuffer per egui frame.

use rack_core::messages::{encode_batch, Msg};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;

use crate::audio::{AudioSystem, MeterMap};

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum LinkState {
    #[default]
    Off,
    Starting,
    Running,
}

#[derive(Default)]
pub struct EngineLink {
    audio: Option<AudioSystem>,
    queue: Vec<Msg>,
    failed: Option<String>,
    /// Filled by the audio system's onmessage handler; survives power cycles
    /// so panels just show the last snapshot until fresh data arrives.
    meters: Rc<RefCell<MeterMap>>,
}

impl EngineLink {
    /// Shared meter store; hand this to `audio::start` and to the rack UI.
    pub fn meters(&self) -> Rc<RefCell<MeterMap>> {
        self.meters.clone()
    }
    pub fn state(&self) -> LinkState {
        match &self.audio {
            None => LinkState::Off,
            Some(a) if a.is_ready() => LinkState::Running,
            Some(_) => LinkState::Starting,
        }
    }

    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn attach(&mut self, audio: AudioSystem) {
        self.failed = None;
        self.audio = Some(audio);
    }

    pub fn fail(&mut self, err: JsValue) {
        self.failed = Some(format!("{err:?}"));
        self.audio = None;
    }

    pub fn power_off(&mut self) {
        if let Some(a) = self.audio.take() {
            // Detach the port handler BEFORE dropping the closure, so a
            // meter message still in flight from the worklet can't invoke a
            // dropped closure ("closure invoked after being dropped").
            a.port.set_onmessage(None);
            let _ = a.ctx.close();
        }
        self.queue.clear();
    }

    /// Queue a control message; sent on the next flush.
    pub fn send(&mut self, msg: Msg) {
        if self.audio.is_some() {
            self.queue.push(msg);
        }
    }

    /// Send a plan blob immediately (plans are not batched with params: they
    /// must arrive before any SetParam that targets their slots).
    pub fn send_plan(&mut self, blob: &[u8]) {
        let Some(audio) = &self.audio else { return };
        let array = js_sys::Uint8Array::from(blob);
        let buffer = array.buffer();
        let payload = js_sys::Object::new();
        if js_sys::Reflect::set(&payload, &"msgs".into(), &buffer).is_ok() {
            let transfer = js_sys::Array::of1(&buffer);
            let _ = audio.port.post_message_with_transferable(&payload, &transfer);
        }
    }

    /// Flush queued messages as one transferable buffer. Call once per frame.
    pub fn flush(&mut self) {
        let Some(audio) = &self.audio else { return };
        if self.queue.is_empty() || !audio.is_ready() {
            return;
        }
        let bytes = encode_batch(&self.queue);
        self.queue.clear();

        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let buffer = array.buffer();
        let payload = js_sys::Object::new();
        if js_sys::Reflect::set(&payload, &"msgs".into(), &buffer).is_ok() {
            let transfer = js_sys::Array::of1(&buffer);
            let _ = audio.port.post_message_with_transferable(&payload, &transfer);
        }
    }
}
