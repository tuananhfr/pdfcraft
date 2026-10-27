//! The office shell's share sheet (`window.erpCanShareFiles`, `window.erpShare`) as the app's
//! [`ShareFn`](pdfcraft_ui_egui::share::ShareFn). Without the shell, or where the browser can't
//! share files, there is none and File ▸ Share… stays hidden.

use eframe::wasm_bindgen::{JsCast, JsValue};

fn function(name: &str) -> Option<(web_sys::Window, js_sys::Function)> {
    let window = web_sys::window()?;
    let f = js_sys::Reflect::get(&window, &JsValue::from_str(name)).ok()?.dyn_into::<js_sys::Function>().ok()?;
    Some((window, f))
}

/// The share service, when the shell says the browser can share PDF files (asked once, at start).
pub fn install() -> Option<pdfcraft_ui_egui::share::ShareFn> {
    let (window, can_share) = function("erpCanShareFiles")?;
    if can_share.call1(&window, &JsValue::from_str(pdfcraft_ui_egui::share::PDF_MIME)).ok()?.as_bool() != Some(true) {
        return None;
    }
    Some(Box::new(|bytes: &[u8], name: &str, mime: &str| {
        // Looked up at the click, in the same call: the share sheet needs the click's activation.
        let Some((window, share)) = function("erpShare") else { return false };
        let bytes = js_sys::Uint8Array::from(bytes);
        share.call3(&window, &bytes, &JsValue::from_str(name), &JsValue::from_str(mime)).ok().and_then(|r| r.as_bool()) == Some(true)
    }))
}
