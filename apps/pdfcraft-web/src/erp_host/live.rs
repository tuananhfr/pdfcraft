//! `erpHost` v3, region co-editing: the app's calls into the shell (`regionWillEdit`,
//! `regionChanged`, `regionDidEndEdit`, `selectionChanged`) and the shell's handlers, queued for
//! the next frame.

use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
use pdfcraft_ui_egui::online::live::{Choice, LiveEvent, LiveInbox, RegionHost};

use super::convert;

/// Whether the shell offers region co-editing (detected by feature: `erpHost.version` stays 2).
pub fn offered(host: &JsValue) -> bool {
    js_sys::Reflect::get(host, &JsValue::from_str("regionWillEdit")).is_ok_and(|f| f.is_function())
}

pub struct RegionBridge {
    host: JsValue,
    ctx: egui::Context,
}

impl RegionBridge {
    pub fn new(host: JsValue, ctx: egui::Context) -> Self {
        Self { host, ctx }
    }

    fn call(&self, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let f: js_sys::Function = js_sys::Reflect::get(&self.host, &JsValue::from_str(method))?.dyn_into()?;
        let args: js_sys::Array = args.iter().collect();
        f.apply(&self.host, &args)
    }
}

impl RegionHost for RegionBridge {
    fn region_will_edit(&self, region: &str, reply: Box<dyn FnOnce(bool)>) {
        let called = self.call("regionWillEdit", &[JsValue::from_str(region)]);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // A throw, a rejection or anything but `true` is a refusal: the change is taken back.
            let granted = match called {
                Ok(value) => wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&value)).await.is_ok_and(|v| v.as_bool() == Some(true)),
                Err(_) => false,
            };
            reply(granted);
            ctx.request_repaint();
        });
    }

    fn region_changed(&self, region: &str, payload: &str) {
        let _ = self.call("regionChanged", &[JsValue::from_str(region), JsValue::from_str(payload)]);
    }

    fn region_did_end_edit(&self, region: &str) {
        let _ = self.call("regionDidEndEdit", &[JsValue::from_str(region)]);
    }

    fn selection_changed(&self, region: &str, range: &str) {
        let _ = self.call("selectionChanged", &[JsValue::from_str(region), JsValue::from_str(range)]);
    }
}

/// The v3 handlers for `ready`: `applyRemote`, `showPresence`, `showLocks`, `showCursors`,
/// `lockDenied`, `isLeader` and `regionConflict`.
pub fn handlers(inbox: &LiveInbox, ctx: &egui::Context) -> Vec<(&'static str, JsValue)> {
    let queue = |inbox: &LiveInbox, ctx: &egui::Context| {
        let (inbox, ctx) = (inbox.clone(), ctx.clone());
        move |event: Option<LiveEvent>| {
            if let Some(event) = event
                && let Ok(mut q) = inbox.lock()
            {
                q.push(event);
            }
            ctx.request_repaint();
        }
    };
    let push = queue(inbox, ctx);
    let apply_remote = Closure::<dyn FnMut(JsValue, JsValue, JsValue)>::new(move |region: JsValue, payload: JsValue, author: JsValue| {
        push(region.as_string().zip(payload.as_string()).map(|(region, payload)| LiveEvent::ApplyRemote {
            region,
            payload,
            author: author.as_string(),
        }));
    })
    .into_js_value();
    let push = queue(inbox, ctx);
    let show_presence =
        Closure::<dyn FnMut(JsValue)>::new(move |members: JsValue| push(Some(LiveEvent::Presence(convert::members(&members))))).into_js_value();
    let push = queue(inbox, ctx);
    let show_locks = Closure::<dyn FnMut(JsValue)>::new(move |locks: JsValue| push(Some(LiveEvent::Locks(convert::locks(&locks))))).into_js_value();
    let push = queue(inbox, ctx);
    let show_cursors =
        Closure::<dyn FnMut(JsValue)>::new(move |cursors: JsValue| push(Some(LiveEvent::Cursors(convert::cursors(&cursors))))).into_js_value();
    let push = queue(inbox, ctx);
    let lock_denied = Closure::<dyn FnMut(JsValue)>::new(move |region: JsValue| push(region.as_string().map(LiveEvent::LockDenied))).into_js_value();
    let push = queue(inbox, ctx);
    let is_leader =
        Closure::<dyn FnMut(JsValue)>::new(move |flag: JsValue| push(Some(LiveEvent::Leader(flag.as_bool() == Some(true))))).into_js_value();
    let push = queue(inbox, ctx);
    let region_conflict = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(move |region: JsValue| {
        let region = region.as_string().unwrap_or_default();
        js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
            let reply = Box::new(move |choice: Choice| {
                let _ = resolve.call1(&JsValue::UNDEFINED, &JsValue::from_str(choice.as_str()));
            });
            push(Some(LiveEvent::Conflict { region: region.clone(), reply }));
        })
    })
    .into_js_value();
    vec![
        ("applyRemote", apply_remote),
        ("showPresence", show_presence),
        ("showLocks", show_locks),
        ("showCursors", show_cursors),
        ("lockDenied", lock_denied),
        ("isLeader", is_leader),
        ("regionConflict", region_conflict),
    ]
}
