//! The `erpHost` comment calls (M5): the shell's handlers `anchorOfSelection`, `showComments`,
//! `focusAnchor` and `resolveAnchors`, and the app's call `commentClicked`. Anchors cross as
//! JavaScript objects and are JSON strings on the app's side.

use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
use pdfcraft_ui_egui::online::comments::{
    CommentEvent, CommentHost, CommentInbox, MAX_ANCHOR_BYTES, MAX_THREADS, ResolvedThread, SelectionAnchor, Thread,
};

use super::convert::bounded;

/// A thread id or status longer than this is no id or status the shell makes.
const MAX_ID_BYTES: usize = 256;

/// Whether the shell has Cloud comments (detected by feature, like v3 regions).
pub fn offered(host: &JsValue) -> bool {
    js_sys::Reflect::get(host, &JsValue::from_str("commentClicked")).is_ok_and(|f| f.is_function())
}

pub struct CommentBridge {
    host: JsValue,
}

impl CommentBridge {
    pub fn new(host: JsValue) -> Self {
        Self { host }
    }
}

impl CommentHost for CommentBridge {
    fn comment_clicked(&self, thread: &str) {
        // A shell without the call gets nothing.
        let Ok(f) = js_sys::Reflect::get(&self.host, &JsValue::from_str("commentClicked")).and_then(|f| f.dyn_into::<js_sys::Function>()) else {
            return;
        };
        let _ = f.call1(&self.host, &JsValue::from_str(thread));
    }
}

fn field(value: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(value, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

/// An anchor as JSON: a string stays as it is, an object is stringified. One longer than the
/// app reads is not copied (the app would refuse it anyway).
fn anchor_json(value: &JsValue) -> Option<String> {
    if value.is_string() {
        return bounded(value, MAX_ANCHOR_BYTES);
    }
    value.is_object().then(|| js_sys::JSON::stringify(value).ok().and_then(|s| bounded(&s, MAX_ANCHOR_BYTES))).flatten()
}

/// An id the shell may give as a string or a number.
fn id(value: &JsValue) -> Option<String> {
    bounded(value, MAX_ID_BYTES).or_else(|| value.as_f64().filter(|n| n.is_finite()).map(|n| n.to_string())).filter(|s| !s.is_empty())
}

/// `[{ id, anchor, status, count }]`, the first [`MAX_THREADS`] (the app reads no more); entries
/// without an id or an anchor are skipped.
fn threads(value: &JsValue) -> Vec<Thread> {
    let Some(array) = value.dyn_ref::<js_sys::Array>() else { return Vec::new() };
    array
        .iter()
        .take(MAX_THREADS)
        .filter_map(|t| {
            Some(Thread {
                id: id(&field(&t, "id")?)?,
                anchor: anchor_json(&field(&t, "anchor")?)?,
                status: field(&t, "status").and_then(|s| bounded(&s, MAX_ID_BYTES)).unwrap_or_default(),
                count: field(&t, "count")
                    .and_then(|c| c.as_f64())
                    .filter(|c| c.is_finite() && *c >= 0.0)
                    .map_or(0, |c| c.min(f64::from(u32::MAX)) as u32),
            })
        })
        .collect()
}

/// An anchor's JSON back as an object; a string that won't parse stays a string.
fn parsed(json: &str) -> JsValue {
    js_sys::JSON::parse(json).unwrap_or_else(|_| JsValue::from_str(json))
}

fn resolved(list: Vec<ResolvedThread>) -> JsValue {
    let out = js_sys::Array::new();
    for r in list {
        let o = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("id"), &JsValue::from_str(&r.id));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("anchor"), &parsed(&r.anchor));
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str("status"), &JsValue::from_str(r.status.as_str()));
        out.push(&o);
    }
    out.into()
}

/// The comment handlers for `ready`.
pub fn handlers(inbox: &CommentInbox, selection: &SelectionAnchor, ctx: &egui::Context) -> Vec<(&'static str, JsValue)> {
    let queue = |inbox: &CommentInbox, ctx: &egui::Context| {
        let (inbox, ctx) = (inbox.clone(), ctx.clone());
        move |event: CommentEvent| {
            if let Ok(mut q) = inbox.lock() {
                q.push(event);
            }
            ctx.request_repaint();
        }
    };
    let selection = selection.clone();
    // Answered at once from the last frame: the shell may ask while its own button has the focus.
    let anchor_of_selection =
        Closure::<dyn FnMut() -> JsValue>::new(move || selection.lock().ok().and_then(|s| s.clone()).map_or(JsValue::NULL, |json| parsed(&json)))
            .into_js_value();
    let push = queue(inbox, ctx);
    let show_comments = Closure::<dyn FnMut(JsValue)>::new(move |list: JsValue| push(CommentEvent::Show(threads(&list)))).into_js_value();
    let push = queue(inbox, ctx);
    let focus_anchor = Closure::<dyn FnMut(JsValue)>::new(move |anchor: JsValue| {
        if let Some(json) = anchor_json(&anchor) {
            push(CommentEvent::Focus(json));
        }
    })
    .into_js_value();
    let push = queue(inbox, ctx);
    let resolve_anchors = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(move |list: JsValue| {
        let threads = threads(&list);
        js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
            let reply = Box::new(move |list: Vec<ResolvedThread>| {
                let _ = resolve.call1(&JsValue::UNDEFINED, &resolved(list));
            });
            push(CommentEvent::Resolve { threads: threads.clone(), reply });
        })
    })
    .into_js_value();
    vec![
        ("anchorOfSelection", anchor_of_selection),
        ("showComments", show_comments),
        ("focusAnchor", focus_anchor),
        ("resolveAnchors", resolve_anchors),
    ]
}
