//! Audio bootstrap: AudioContext + worklet module + engine wasm handshake.
//! Mirrors what web/index.html does in JS, driven from the egui app.

use rack_core::meters::{decode_meters, MeterEntry};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, AudioWorkletNode, AudioWorkletNodeOptions, MessagePort, Response};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Latest meter snapshot per engine slot, refreshed ~30 Hz by the worklet.
pub type MeterMap = HashMap<u16, MeterEntry>;

/// Result of a successful bootstrap. Keeps the JS objects (and the onmessage
/// closure) alive for the lifetime of the app.
pub struct AudioSystem {
    pub ctx: AudioContext,
    // Kept alive for the app's lifetime; sample_rate feeds MIDI clock
    // conversion in phase 6.
    #[allow(dead_code)]
    pub node: AudioWorkletNode,
    pub port: MessagePort,
    #[allow(dead_code)]
    pub sample_rate: f32,
    ready: Rc<Cell<bool>>,
    _onmessage: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

impl AudioSystem {
    /// True once the worklet has instantiated the engine wasm.
    pub fn is_ready(&self) -> bool {
        self.ready.get()
    }
}

pub async fn start(meters: Rc<RefCell<MeterMap>>) -> Result<AudioSystem, JsValue> {
    let ctx = AudioContext::new()?;
    // Must follow a user gesture; egui's click satisfies Chrome's sticky
    // user-activation requirement.
    JsFuture::from(ctx.resume()?).await?;
    JsFuture::from(ctx.audio_worklet()?.add_module("./worklet-shim.js")?).await?;

    // Fetch the engine wasm and ship raw bytes to the worklet (the worklet
    // scope has no fetch; compiled-Module cloning is unreliable there). A
    // per-load cache-buster query forces a fresh download so a rebuilt engine
    // (e.g. new module kinds) is never masked by a stale cached worklet wasm.
    let window = web_sys::window().ok_or("no window")?;
    let url = format!("./worklet/rack_worklet_bg.wasm?v={}", js_sys::Date::now() as u64);
    let resp: Response =
        JsFuture::from(window.fetch_with_str(&url)).await?.dyn_into()?;
    let wasm_bytes = JsFuture::from(resp.array_buffer()?).await?;

    let opts = AudioWorkletNodeOptions::new();
    opts.set_number_of_inputs(0);
    opts.set_number_of_outputs(1);
    let channel_counts = js_sys::Array::of1(&JsValue::from_f64(2.0));
    opts.set_output_channel_count(&channel_counts);
    let node = AudioWorkletNode::new_with_options(&ctx, "rack-processor", &opts)?;
    let port = node.port()?;

    let ready = Rc::new(Cell::new(false));
    let onmessage = {
        let ready = ready.clone();
        Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
            let data = e.data();
            // Meter snapshots are the hot path: check them first.
            if let Ok(m) = js_sys::Reflect::get(&data, &"meters".into()) {
                if !m.is_undefined() {
                    let bytes = js_sys::Uint8Array::new(&m).to_vec();
                    // try_borrow_mut, never borrow_mut: if the egui frame is
                    // mid-read of the map, skip this snapshot rather than
                    // panic. A panic would abort the whole wasm instance
                    // (panic = "abort"), killing every closure.
                    if let (Some(entries), Ok(mut map)) =
                        (decode_meters(&bytes), meters.try_borrow_mut())
                    {
                        map.clear();
                        for entry in entries {
                            map.insert(entry.slot, *entry);
                        }
                    }
                    return;
                }
            }
            if js_sys::Reflect::get(&data, &"ready".into()).map(|v| v.is_truthy()).unwrap_or(false) {
                ready.set(true);
            }
            if let Ok(err) = js_sys::Reflect::get(&data, &"error".into()) {
                if err.is_truthy() {
                    web_sys::console::error_2(&"worklet engine error:".into(), &err);
                }
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>)
    };
    port.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

    let handshake = js_sys::Object::new();
    js_sys::Reflect::set(&handshake, &"wasmBytes".into(), &wasm_bytes)?;
    let transfer = js_sys::Array::of1(&wasm_bytes);
    port.post_message_with_transferable(&handshake, &transfer)?;

    node.connect_with_audio_node(&ctx.destination())?;

    let sample_rate = ctx.sample_rate();
    Ok(AudioSystem { ctx, node, port, sample_rate, ready, _onmessage: onmessage })
}
