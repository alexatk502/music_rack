//! Web MIDI input: note on/off from hardware controllers, forwarded into the
//! same message queue as the keyboard piano. Chrome/Edge only — Safari has
//! never shipped Web MIDI, so everything feature-detects.

use rack_core::messages::Msg;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

pub struct MidiState {
    /// Note messages parsed since the last drain (main thread only).
    pending: Vec<Msg>,
    /// Latest value (0..127) received for each Control Change number.
    pub cc: [u8; 128],
    pub device_count: usize,
    pub error: Option<String>,
}

impl Default for MidiState {
    fn default() -> Self {
        Self { pending: Vec::new(), cc: [0; 128], device_count: 0, error: None }
    }
}

pub struct MidiSystem {
    /// Keeps the shared state (and the closures below that write to it)
    /// alive for the app's lifetime.
    #[allow(dead_code)]
    state: Rc<RefCell<MidiState>>,
    /// Keeps the onmidimessage closures alive.
    _handlers: Vec<Closure<dyn FnMut(web_sys::MidiMessageEvent)>>,
}

pub fn supported() -> bool {
    web_sys::window()
        .map(|w| js_sys::Reflect::has(&w.navigator(), &"requestMIDIAccess".into()).unwrap_or(false))
        .unwrap_or(false)
}

pub async fn start(state: Rc<RefCell<MidiState>>) -> Result<MidiSystem, JsValue> {
    let navigator = web_sys::window().ok_or("no window")?.navigator();
    let access: web_sys::MidiAccess =
        JsFuture::from(navigator.request_midi_access()?).await?.dyn_into()?;

    let mut handlers = Vec::new();
    let inputs = access.inputs();
    let mut count = 0usize;
    // MidiInputMap is a JS Map; iterate its values.
    let values = js_sys::try_iter(&inputs.values())?.ok_or("inputs not iterable")?;
    for value in values {
        let input: web_sys::MidiInput = value?.dyn_into()?;
        let state = state.clone();
        let handler = Closure::wrap(Box::new(move |e: web_sys::MidiMessageEvent| {
            let Ok(data) = e.data() else { return };
            if data.len() < 3 {
                return;
            }
            let (status, a, b) = (data[0] & 0xF0, data[1], data[2]);
            let mut state = state.borrow_mut();
            match status {
                0x90 if b > 0 => state.pending.push(Msg::note_on(a, b, 0)),
                0x90 | 0x80 => state.pending.push(Msg::note_off(a, 0)),
                0xB0 => state.cc[(a & 0x7F) as usize] = b, // Control Change
                _ => {}
            }
        }) as Box<dyn FnMut(web_sys::MidiMessageEvent)>);
        input.set_onmidimessage(Some(handler.as_ref().unchecked_ref()));
        handlers.push(handler);
        count += 1;
    }
    state.borrow_mut().device_count = count;
    Ok(MidiSystem { state, _handlers: handlers })
}

impl MidiState {
    pub fn drain(&mut self) -> Vec<Msg> {
        std::mem::take(&mut self.pending)
    }
}
