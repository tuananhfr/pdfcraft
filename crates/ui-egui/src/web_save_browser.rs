//! The browser side of `web_save`: `showSaveFilePicker` and `FileSystemFileHandle` writes where
//! the browser has them (looked up at every call, through `Reflect`), anchor downloads elsewhere.
//! The ERPCons office shell's embed bridge replaces `showSaveFilePicker` with its own handle and
//! answers `close()` only once the ERP has stored the file.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};

use crate::web_save::{BrowserSave, FileId, SaveDone, SaveOutcome};

/// The file handles the picker returned, by the id handed to the app.
#[derive(Default)]
pub(crate) struct Browser {
    files: Rc<RefCell<HashMap<FileId, JsValue>>>,
    next: Cell<FileId>,
}

impl BrowserSave for Browser {
    fn has_picker(&self) -> bool {
        picker().is_some()
    }

    fn pick_and_write(&self, name: &str, bytes: &[u8], done: SaveDone) {
        let opened = picker().ok_or_else(|| "showSaveFilePicker is gone".to_string()).and_then(|p| open_picker(&p, name));
        let promise = match opened {
            Ok(promise) => promise,
            // Thrown before any picker showed (a frame that may not show one): download instead.
            Err(e) => {
                log::warn!(
                    "showSaveFilePicker failed: {}; downloading instead",
                    if name.is_empty() { e } else { e.replace(name, &crate::log_safe::document(name, None)) }
                );
                done(match crate::editing::download(name, bytes) {
                    Ok(()) => SaveOutcome::Saved { name: name.to_string(), file: None },
                    Err(e) => SaveOutcome::Failed(e),
                });
                return;
            }
        };
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        let files = self.files.clone();
        let (name, bytes) = (name.to_string(), bytes.to_vec());
        wasm_bindgen_futures::spawn_local(async move {
            let outcome = match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(handle) => match write(&handle, &bytes).await {
                    Ok(()) => {
                        let picked = handle_name(&handle).unwrap_or(name);
                        if let Ok(mut f) = files.try_borrow_mut() {
                            f.insert(id, handle);
                        }
                        SaveOutcome::Saved { name: picked, file: Some(id) }
                    }
                    Err(e) => SaveOutcome::Failed(e),
                },
                Err(e) if error_name(&e) == "AbortError" => SaveOutcome::Cancelled,
                Err(e) => SaveOutcome::Failed(error_message(&e)),
            };
            done(outcome);
        });
    }

    fn write_to(&self, file: FileId, name: &str, bytes: &[u8], done: SaveDone) {
        let Some(handle) = self.files.try_borrow().ok().and_then(|f| f.get(&file).cloned()) else {
            done(SaveOutcome::Failed("the file chosen earlier is no longer available".into()));
            return;
        };
        let (name, bytes) = (name.to_string(), bytes.to_vec());
        wasm_bindgen_futures::spawn_local(async move {
            done(match write(&handle, &bytes).await {
                Ok(()) => SaveOutcome::Saved { name: handle_name(&handle).unwrap_or(name), file: Some(file) },
                Err(e) => SaveOutcome::Failed(e),
            });
        });
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        crate::editing::download(name, bytes)
    }
}

fn picker() -> Option<js_sys::Function> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str("showSaveFilePicker")).ok()?.dyn_into().ok()
}

/// Show the picker for a PDF named `name`. Called while the click or key that asked for the save
/// still counts as a user gesture; the promise resolves to the chosen file's handle.
fn open_picker(picker: &js_sys::Function, name: &str) -> Result<js_sys::Promise, String> {
    let js = |e: JsValue| error_message(&e);
    let key = JsValue::from_str;
    let accept = js_sys::Object::new();
    js_sys::Reflect::set(&accept, &key("application/pdf"), &js_sys::Array::of1(&key(".pdf"))).map_err(js)?;
    let file_type = js_sys::Object::new();
    js_sys::Reflect::set(&file_type, &key("description"), &key("PDF")).map_err(js)?;
    js_sys::Reflect::set(&file_type, &key("accept"), &accept).map_err(js)?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &key("suggestedName"), &key(name)).map_err(js)?;
    js_sys::Reflect::set(&options, &key("types"), &js_sys::Array::of1(&file_type)).map_err(js)?;
    let window = web_sys::window().ok_or("no window")?;
    picker.call1(&window, &options).map_err(js)?.dyn_into::<js_sys::Promise>().map_err(|_| "not a promise".to_string())
}

/// Write `bytes` to a picked file and close it: only a resolved `close()` means the file is there.
async fn write(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: JsValue| error_message(&e);
    let method = |target: &JsValue, name: &str| -> Result<js_sys::Function, String> {
        js_sys::Reflect::get(target, &JsValue::from_str(name)).map_err(js)?.dyn_into().map_err(|_| format!("{name} is not a function"))
    };
    let promise = |v: JsValue| v.dyn_into::<js_sys::Promise>().map_err(|_| "not a promise".to_string());
    let stream = method(handle, "createWritable")?.call0(handle).map_err(js)?;
    let stream = wasm_bindgen_futures::JsFuture::from(promise(stream)?).await.map_err(js)?;
    let written = method(&stream, "write")?.call1(&stream, &js_sys::Uint8Array::from(bytes)).map_err(js)?;
    if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise(written)?).await {
        // An unclosed stream keeps a swap file around; abort drops it and leaves the old file.
        if let Ok(abort) = method(&stream, "abort")
            && let Ok(p) = abort.call0(&stream)
            && let Ok(p) = promise(p)
        {
            let _ = wasm_bindgen_futures::JsFuture::from(p).await;
        }
        return Err(js(e));
    }
    let closed = method(&stream, "close")?.call0(&stream).map_err(js)?;
    wasm_bindgen_futures::JsFuture::from(promise(closed)?).await.map_err(js)?;
    Ok(())
}

fn handle_name(handle: &JsValue) -> Option<String> {
    js_sys::Reflect::get(handle, &JsValue::from_str("name")).ok()?.as_string().filter(|n| !n.trim().is_empty())
}

/// A DOMException's `name` (`AbortError` when the user closes the picker).
fn error_name(e: &JsValue) -> String {
    js_sys::Reflect::get(e, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

fn error_message(e: &JsValue) -> String {
    js_sys::Reflect::get(e, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|m| !m.is_empty())
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}
