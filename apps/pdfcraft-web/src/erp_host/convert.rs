//! The shell's JavaScript objects as the app's types: `open`'s document and `save`'s `SaveResult`.

use eframe::wasm_bindgen::{JsCast, JsValue};
use pdfcraft_ui_egui::online::{Access, HostDocument, SaveResult};

fn field(value: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(value, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

/// `{ docId, version, title, format, bytes }`, and with `v2` also `mode`, `canDownload` and
/// `preserveView`; `None` without a `docId` (there is no one to answer).
pub fn document(doc: &JsValue, v2: bool) -> Option<HostDocument> {
    let doc_id = field(doc, "docId")?.as_string().filter(|id| !id.is_empty())?;
    let bytes = match field(doc, "bytes") {
        Some(b) if b.is_instance_of::<js_sys::Uint8Array>() => b.unchecked_into::<js_sys::Uint8Array>().to_vec(),
        Some(b) if b.is_instance_of::<js_sys::ArrayBuffer>() => js_sys::Uint8Array::new(&b).to_vec(),
        // Opening fails on no bytes, and the shell hears `opened(docId, false)`.
        _ => Vec::new(),
    };
    let title = field(doc, "title").and_then(|t| t.as_string()).unwrap_or_default();
    let format = field(doc, "format").and_then(|f| f.as_string()).unwrap_or_default();
    let document = HostDocument::new(doc_id, field(doc, "version").and_then(|v| v.as_f64()), title, format, bytes);
    if !v2 {
        return Some(document);
    }
    let preserve_view = field(doc, "preserveView").and_then(|p| p.as_bool()).unwrap_or(false);
    Some(document.with_access(access(field(doc, "mode"), field(doc, "canDownload")), preserve_view))
}

/// `mode` and `canDownload`, as `open` and `setMode` give them.
pub fn access(mode: Option<JsValue>, can_download: Option<JsValue>) -> Access {
    // A mode that is there but not a word never grants editing.
    let mode = mode.map(|m| m.as_string().unwrap_or_default());
    Access::new(mode.as_deref(), can_download.and_then(|c| c.as_bool()))
}

/// The account's name from `setUser`'s `{ id, name }`; `None` for `null` or no name.
pub fn user_name(user: &JsValue) -> Option<String> {
    if !user.is_object() {
        return None;
    }
    field(user, "name").and_then(|n| n.as_string())
}

/// A field of a JavaScript call's argument; `undefined` and `null` are absent.
pub fn present(value: JsValue) -> Option<JsValue> {
    (!value.is_undefined() && !value.is_null()).then_some(value)
}

/// What `erpHost.save` resolved to.
pub fn save_result(result: &JsValue) -> SaveResult {
    SaveResult::from_fields(
        field(result, "status").and_then(|s| s.as_string()).as_deref(),
        field(result, "version").and_then(|v| v.as_f64()),
        field(result, "latestVersion").and_then(|v| v.as_f64()),
        field(result, "retryable").and_then(|r| r.as_bool()),
        field(result, "message").and_then(|m| m.as_string()),
    )
}
