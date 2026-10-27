//! `window.erpHost` v1 of the ERPCons office shell (`docs/erp-host.md` in erpcons-office): documents
//! the office server holds, opened with `open(doc)` and saved back through `erpHost.save`. The app's
//! side is [`pdfcraft_ui_egui::online`]; this is the JavaScript glue.

mod bridge;
mod convert;

use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
use pdfcraft_ui_egui::online::{Acknowledged, FLUSH_TIMEOUT_MS, Online};

/// The shell's `erpHost`, when it offers version 1 (checked once, at start).
fn host() -> Option<JsValue> {
    let window = web_sys::window()?;
    let host = js_sys::Reflect::get(&window, &JsValue::from_str("erpHost")).ok().filter(JsValue::is_object)?;
    let version = js_sys::Reflect::get(&host, &JsValue::from_str("version")).ok()?.as_f64();
    (version == Some(1.0)).then_some(host)
}

/// Register `open`, `flush` and `acknowledged` with the shell. `None` without `erpHost` v1: the app works as before.
pub fn install(ctx: &egui::Context) -> Option<Online> {
    let host = host()?;
    let ready: js_sys::Function = js_sys::Reflect::get(&host, &JsValue::from_str("ready")).ok()?.dyn_into().ok()?;
    let online = Online::new(Box::new(bridge::Bridge::new(host.clone(), ctx.clone())));
    let inbox = online.inbox();
    let repaint = ctx.clone();
    // Only queued here: the document opens on the next frame, never inside the callback.
    let open = Closure::<dyn FnMut(JsValue)>::new(move |doc: JsValue| {
        if let Some(doc) = convert::document(&doc)
            && let Ok(mut q) = inbox.lock()
        {
            q.push(doc);
        }
        repaint.request_repaint();
    })
    .into_js_value();
    let handle = online.flush_handle();
    let flush = Closure::<dyn FnMut() -> js_sys::Promise>::new(move || {
        let handle = handle.clone();
        js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
            // Settles on whichever comes first: every save handed over, or the time limit.
            let once = std::rc::Rc::new(std::cell::Cell::new(Some(resolve)));
            let on_time = once.clone();
            let settle = move || {
                if let Some(resolve) = once.take() {
                    let _ = resolve.call0(&JsValue::UNDEFINED);
                }
            };
            let timeout = Closure::once_into_js(move || {
                if let Some(resolve) = on_time.take() {
                    let _ = resolve.call0(&JsValue::UNDEFINED);
                }
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    timeout.unchecked_ref(),
                    i32::try_from(FLUSH_TIMEOUT_MS).unwrap_or(i32::MAX),
                );
            }
            handle.flush(Box::new(settle));
        })
    })
    .into_js_value();
    let acks = online.acknowledgements();
    let repaint = ctx.clone();
    let acknowledged = Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |doc_id: JsValue, revision: JsValue, version: JsValue| {
        if let (Some(doc_id), Some(revision)) = (doc_id.as_string(), revision.as_f64())
            && let Some(ack) = Acknowledged::new(doc_id, revision, version.as_f64())
            && let Ok(mut q) = acks.lock()
        {
            q.push(ack);
        }
        repaint.request_repaint();
    })
    .into_js_value();
    let handlers = js_sys::Object::new();
    for (key, value) in [("open", &open), ("flush", &flush), ("acknowledged", &acknowledged)] {
        js_sys::Reflect::set(&handlers, &JsValue::from_str(key), value).ok()?;
    }
    ready.call1(&host, &handlers).ok()?;
    Some(online)
}
