//! Patch persistence: localStorage autosave, file export (Blob download),
//! and import via files dropped onto the window.

use rack_graph::persist;
use rack_graph::subpatch::{self, CustomModule};
use rack_graph::Patch;
use wasm_bindgen::JsCast;

const STORAGE_KEY: &str = "music_rack_patch";
const CUSTOMS_KEY: &str = "music_rack_customs";

/// Load the saved custom-module library from localStorage.
pub fn load_customs() -> Vec<CustomModule> {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return Vec::new();
    };
    match storage.get_item(CUSTOMS_KEY) {
        Ok(Some(json)) => subpatch::customs_from_json(&json),
        _ => Vec::new(),
    }
}

/// Persist the custom-module library to localStorage.
pub fn save_customs(customs: &[CustomModule]) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return;
    };
    let _ = storage.set_item(CUSTOMS_KEY, &subpatch::customs_to_json(customs));
}

pub fn save_to_local_storage(patch: &Patch) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return;
    };
    let _ = storage.set_item(STORAGE_KEY, &persist::to_json(patch));
}

pub fn load_from_local_storage() -> Option<Patch> {
    let storage = web_sys::window()?.local_storage().ok()??;
    let json = storage.get_item(STORAGE_KEY).ok()??;
    match persist::from_json(&json) {
        Ok(patch) => Some(patch),
        Err(e) => {
            web_sys::console::warn_1(&format!("stored patch failed to load: {e}").into());
            None
        }
    }
}

/// Trigger a browser download of the patch JSON.
pub fn export_download(patch: &Patch) {
    let json = persist::to_json(patch);
    let result = (|| -> Result<(), wasm_bindgen::JsValue> {
        let array = js_sys::Array::of1(&js_sys::JsString::from(json.as_str()));
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&array, &{
            let opts = web_sys::BlobPropertyBag::new();
            opts.set_type("application/json");
            opts
        })?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let document = web_sys::window().ok_or("no window")?.document().ok_or("no document")?;
        let a: web_sys::HtmlAnchorElement =
            document.create_element("a")?.dyn_into().map_err(|_| "anchor cast")?;
        a.set_href(&url);
        a.set_download("patch.json");
        a.click();
        web_sys::Url::revoke_object_url(&url)?;
        Ok(())
    })();
    if let Err(e) = result {
        web_sys::console::error_2(&"patch export failed:".into(), &e);
    }
}

/// Parse a dropped .json file's bytes into a patch.
pub fn import_bytes(bytes: &[u8]) -> Result<Patch, String> {
    let json = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    persist::from_json(json)
}
