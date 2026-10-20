//! Browser clipboard access; promises start during the command's user gesture.

use eframe::wasm_bindgen::{JsCast, JsValue};
use pdfcraft_ui_egui::clipboard::{Contents, HostClipboard, ReadInbox};
use wasm_bindgen_futures::{JsFuture, spawn_local};

const MAX_BYTES: f64 = 64.0 * 1024.0 * 1024.0;

fn property(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    js_sys::Reflect::get(object, &name.into())
}

fn call(object: &JsValue, name: &str, args: &js_sys::Array) -> Result<JsValue, JsValue> {
    property(object, name)?.dyn_into::<js_sys::Function>()?.apply(object, args)
}

fn clipboard() -> Result<JsValue, JsValue> {
    let window = web_sys::window().ok_or(JsValue::NULL)?;
    property(&property(window.as_ref(), "navigator")?, "clipboard")
}

async fn resolved(value: JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(js_sys::Promise::resolve(&value)).await
}

async fn read_items(pending: Result<JsValue, JsValue>) -> Result<Option<Contents>, JsValue> {
    let items = js_sys::Array::from(&resolved(pending?).await?);
    for mime in ["image/png", "image/jpeg", "text/plain"] {
        for item in items.iter() {
            let types = js_sys::Array::from(&property(&item, "types")?);
            if !types.iter().any(|value| value.as_string().as_deref() == Some(mime)) {
                continue;
            }
            let blob = resolved(call(&item, "getType", &js_sys::Array::of1(&mime.into()))?).await?;
            let size = property(&blob, "size")?.as_f64().ok_or(JsValue::NULL)?;
            if !size.is_finite() || !(0.0..=MAX_BYTES).contains(&size) {
                return Err(JsValue::NULL);
            }
            let bytes = resolved(call(&blob, "arrayBuffer", &js_sys::Array::new())?).await?;
            let bytes = js_sys::Uint8Array::new(&bytes).to_vec();
            if mime == "text/plain" {
                let text = String::from_utf8(bytes).map_err(|_| JsValue::NULL)?;
                if !text.trim().is_empty() {
                    return Ok(Some(Contents::Text(text)));
                }
            } else {
                let name = if mime == "image/png" { "Clipboard.png" } else { "Clipboard.jpeg" };
                return Ok(Some(Contents::Image { name: name.into(), bytes }));
            }
        }
    }
    Ok(None)
}

fn rich_write(clipboard: &JsValue, text: &str) -> Result<JsValue, JsValue> {
    let window = web_sys::window().ok_or(JsValue::NULL)?;
    let constructor = property(window.as_ref(), "ClipboardItem")?.dyn_into::<js_sys::Function>()?;
    let blob_constructor = property(window.as_ref(), "Blob")?.dyn_into::<js_sys::Function>()?;
    let data = js_sys::Object::new();
    for (mime, contents) in [("text/plain", text.to_string()), ("text/html", super::clipboard_html::paragraphs(text))] {
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"type".into(), &mime.into())?;
        let blob = js_sys::Reflect::construct(&blob_constructor, &js_sys::Array::of2(&js_sys::Array::of1(&contents.into()), &options))?;
        js_sys::Reflect::set(&data, &mime.into(), &blob)?;
    }
    let item = js_sys::Reflect::construct(&constructor, &js_sys::Array::of1(&data))?;
    call(clipboard, "write", &js_sys::Array::of1(&js_sys::Array::of1(&item)))
}

fn write(text: &str) {
    let Ok(clipboard) = clipboard() else { return };
    let pending = rich_write(&clipboard, text);
    // Unsupported ClipboardItem falls back before yielding, retaining the user activation.
    let pending = pending.or_else(|_| call(&clipboard, "writeText", &js_sys::Array::of1(&text.into())));
    let text = text.to_string();
    spawn_local(async move {
        if let Ok(pending) = pending
            && resolved(pending).await.is_err()
            && let Ok(fallback) = call(&clipboard, "writeText", &js_sys::Array::of1(&text.into()))
        {
            let _ = resolved(fallback).await;
        }
    });
}

pub fn install(ctx: egui::Context) -> HostClipboard {
    let inbox = ReadInbox::default();
    let queue = inbox.clone();
    HostClipboard {
        inbox,
        write: Box::new(write),
        read: Box::new(move || {
            let pending = clipboard().and_then(|c| call(&c, "read", &js_sys::Array::new()));
            let queue = queue.clone();
            let ctx = ctx.clone();
            spawn_local(async move {
                let result = read_items(pending).await.map_err(|_| ());
                if let Ok(mut q) = queue.lock() {
                    q.push(result);
                }
                ctx.request_repaint();
            });
        }),
    }
}
