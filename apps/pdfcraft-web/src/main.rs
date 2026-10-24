//! PdfCraft in the browser. Build: `cd apps/pdfcraft-web && trunk build --release`
//! (trunk generates the JS loader; no handwritten JS — plan/execution-plan.md §1).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_arch = "wasm32", test))]
mod account;

#[cfg(any(target_arch = "wasm32", test))]
mod draft_state;

#[cfg(any(target_arch = "wasm32", test))]
fn web_app() -> pdfcraft_ui_egui::PdfCraftApp {
    let mut app = pdfcraft_ui_egui::PdfCraftApp::new();
    app.branding = Some(pdfcraft_ui_egui::branding::erpcons());
    app
}

#[cfg(test)]
mod branding_tests {
    #[test]
    fn web_startup_installs_product_publisher_and_website() {
        let app = super::web_app();
        assert_eq!(app.product_name(), "PDF – ERPCons");
        let branding = app.branding.unwrap();
        assert_eq!(branding.publisher, "ERPCons");
        assert_eq!(branding.website, "https://office.erpcons.ai");
        assert!(include_str!("../index.html").contains("<title>PDF – ERPCons</title>"));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    let options = eframe::WebOptions { renderer: eframe::Renderer::Glow, ..Default::default() };
    wasm_bindgen_futures::spawn_local(async move {
        let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("pdfcraft"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return;
        };
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(|cc| {
                    let mut app = web_app();
                    app.clipboard = Some(clipboard::install(cc.egui_ctx.clone()));
                    // "printcraft": settings saved under the app's former name, PrintCraft.
                    if let Some(json) = cc.storage.and_then(|s| s.get_string("pdfcraft").or_else(|| s.get_string("printcraft"))) {
                        app.restore(&json);
                    }
                    // `?file=<url>` opens a PDF from a URL (same-origin or CORS-enabled).
                    if let Some(url) = query_param("file") {
                        let inbox = app.startup_inbox.clone();
                        let failed = app.failed_inbox.clone();
                        let ctx = cc.egui_ctx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let name = url.rsplit('/').next().unwrap_or("document.pdf").split('?').next().unwrap_or("document.pdf").to_string();
                            match fetch_bytes(&url).await {
                                Ok(bytes) => {
                                    if let Ok(mut q) = inbox.lock() {
                                        q.push((name, bytes));
                                    }
                                }
                                Err(e) => {
                                    eframe::web_sys::console::error_1(&format!("PdfCraft: could not fetch {url}: {e}").into());
                                    // Shown in the app too, not only in the console (#173).
                                    if let Ok(mut q) = failed.lock() {
                                        q.push((name, e));
                                    }
                                }
                            }
                            ctx.request_repaint();
                        });
                    }
                    // eframe never raises a close request on a browser reload or tab close, so `guard_quit` never
                    // prompts; ask the browser to confirm instead while a document has unsaved work (#812).
                    warn_before_unload(app.unsaved_flag.clone());
                    // Draft autosave into the ERPCons office shell's store (`window.erpDrafts`);
                    // an embedding host keeps the files itself, so not there.
                    if query_param("host").as_deref() != Some("parent") {
                        app.drafts = Some(drafts::install(cc.egui_ctx.clone()));
                    }
                    // The office shell's PI ID session, when it offers one here (not in embed mode).
                    if let Some(account) = account::install() {
                        app.account_hook = Some(std::sync::Arc::new(account));
                    }
                    // `?host=parent`: a same-origin page embedding PdfCraft in an iframe (such
                    // as the Nextcloud app) passes files in and takes saves back (`host` below).
                    if query_param("host").as_deref() == Some("parent")
                        && let Some(bridge) = host::Bridge::new()
                    {
                        // `?author=`: the host's name for the user signs their comments,
                        // unless they chose a name in PdfCraft (the web default is "Guest").
                        if let Some(author) = query_param("author").map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
                            && app.comment_prefs.author == "Guest"
                        {
                            app.comment_prefs.author = author.chars().take(200).collect();
                        }
                        bridge.listen(app.startup_inbox.clone(), cc.egui_ctx.clone());
                        app.host_save = Some(bridge.save_fn());
                        app.host_dirty = Some(bridge.dirty_fn());
                        bridge.post_ready();
                    }
                    Ok(Box::new(language::SiteLanguage::new(app)))
                }),
            )
            .await;
        if let Err(e) = started {
            eframe::web_sys::console::error_1(&e);
            if query_param("host").as_deref() == Some("parent")
                && let Some(bridge) = host::Bridge::new()
            {
                bridge.post_failed(&e.as_string().unwrap_or_else(|| format!("{e:?}")));
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn warn_before_unload(unsaved: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use eframe::wasm_bindgen::{JsCast, closure::Closure};
    let Some(window) = web_sys::window() else { return };
    let handler = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if unsaved.load(std::sync::atomic::Ordering::Relaxed) {
            event.prevent_default();
        }
    });
    if window.add_event_listener_with_callback("beforeunload", handler.as_ref().unchecked_ref()).is_ok() {
        handler.forget();
    }
}

#[cfg(target_arch = "wasm32")]
mod language;

#[cfg(target_arch = "wasm32")]
mod clipboard;
#[cfg(any(target_arch = "wasm32", test))]
mod clipboard_html;

/// The JavaScript side of [`pdfcraft_ui_egui::drafts`]: `window.erpDrafts.{put, remove, ready}`
/// of the ERPCons office shell, looked up at every call (absent: the feature is off), and the
/// flush when the page is hidden or left.
#[cfg(target_arch = "wasm32")]
mod drafts {
    use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use pdfcraft_ui_egui::drafts::{DraftInbox, DraftRestore, DraftSink, Drafts, FlushCell};

    #[derive(Clone)]
    struct WebSink {
        tracking: std::rc::Rc<std::cell::RefCell<Tracking>>,
        ctx: egui::Context,
        /// `restore(key, name, format, bytes)`, handed to `ready`.
        restore: JsValue,
    }

    #[derive(Default)]
    struct Tracking {
        states: crate::draft_state::States,
        generations: std::collections::BTreeMap<String, u64>,
        failed: Vec<(String, u64)>,
    }

    fn state_available() -> bool {
        store().and_then(|s| js_sys::Reflect::get(&s, &"state".into()).ok()).is_some_and(|v| v.is_function())
    }

    fn send(messages: Vec<crate::draft_state::Message>) {
        let Some(store) = store() else { return };
        let Ok(function) = js_sys::Reflect::get(&store, &"state".into()) else { return };
        let Some(function) = function.dyn_ref::<js_sys::Function>() else { return };
        for m in messages {
            let value = js_sys::Object::new();
            let fields = [
                ("docId", JsValue::from_str(&m.doc_id)),
                ("name", JsValue::from_str(&m.name)),
                ("revision", JsValue::from_f64(m.revision as f64)),
                ("dirty", JsValue::from_bool(m.dirty)),
                ("pending", JsValue::from_bool(m.pending)),
                ("committed", JsValue::from_bool(m.committed)),
                ("failed", JsValue::from_bool(m.failed)),
            ];
            if fields.into_iter().all(|(key, v)| js_sys::Reflect::set(&value, &JsValue::from_str(key), &v).is_ok()) {
                let _ = function.call1(&store, &value);
            }
        }
    }

    fn store() -> Option<JsValue> {
        let window = web_sys::window()?;
        js_sys::Reflect::get(&window, &JsValue::from_str("erpDrafts")).ok().filter(|s| s.is_object())
    }

    fn call(method: &str, args: &[JsValue]) -> Result<JsValue, String> {
        let store = store().ok_or("window.erpDrafts is gone")?;
        let f: js_sys::Function = js_sys::Reflect::get(&store, &JsValue::from_str(method))
            .ok()
            .and_then(|f| f.dyn_into().ok())
            .ok_or_else(|| format!("erpDrafts.{method} is not a function"))?;
        let args: js_sys::Array = args.iter().collect();
        f.apply(&store, &args).map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    impl DraftSink for WebSink {
        fn available(&self) -> bool {
            store().is_some()
        }

        fn ready(&self) -> Result<(), String> {
            call("ready", std::slice::from_ref(&self.restore)).map(|_| ())
        }

        fn put(&self, key: &str, name: &str, format: &str, bytes: &[u8]) -> Result<(), String> {
            // A JS-owned copy: a view into wasm memory would go stale when memory grows.
            let bytes = js_sys::Uint8Array::from(bytes);
            let revision = if let Ok(mut tracking) = self.tracking.try_borrow_mut() {
                send(tracking.states.checkpoint(key));
                tracking.states.revision(key)
            } else {
                None
            };
            let result = call("put", &[key.into(), name.into(), format.into(), bytes.into()]);
            if !state_available() {
                return result.map(|_| ());
            }
            let Some(revision) = revision else { return result.map(|_| ()) };
            let sink = self.clone();
            let key = key.to_owned();
            wasm_bindgen_futures::spawn_local(async move {
                let success = match result {
                    Ok(value) => match value.dyn_into::<js_sys::Promise>() {
                        Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| v.as_bool()) == Some(true),
                        Err(_) => false,
                    },
                    Err(_) => false,
                };
                if let Ok(mut tracking) = sink.tracking.try_borrow_mut() {
                    send(tracking.states.complete(&key, revision, success));
                    if !success {
                        tracking.failed.push((key, revision));
                    }
                }
                sink.ctx.request_repaint();
            });
            Ok(())
        }

        fn remove(&self, key: &str) -> Result<(), String> {
            call("remove", &[key.into()]).map(|_| ())
        }

        fn observe(&self, documents: &[pdfcraft_ui_egui::drafts::DraftDocument], active: Option<&str>) {
            if !state_available() {
                return;
            }
            let Ok(mut tracking) = self.tracking.try_borrow_mut() else { return };
            let closed: Vec<_> = tracking.generations.keys().filter(|key| !documents.iter().any(|d| &d.key == *key)).cloned().collect();
            send(tracking.states.switch(active));
            for key in closed {
                send(tracking.states.close(&key));
                tracking.generations.remove(&key);
            }
            for d in documents {
                send(tracking.states.observe(&d.key, &d.name, d.generation, d.unsaved));
                tracking.generations.insert(d.key.clone(), d.generation);
            }
        }

        fn restored(&self, key: &str, name: &str, generation: u64) {
            if !state_available() {
                return;
            }
            if let Ok(mut tracking) = self.tracking.try_borrow_mut() {
                tracking.states.observe(key, name, generation, true);
                tracking.states.checkpoint(key);
                send(tracking.states.complete(key, generation, true));
                tracking.generations.insert(key.to_owned(), generation);
            }
        }

        fn failed(&self) -> Vec<(String, u64)> {
            self.tracking.try_borrow_mut().map(|mut t| std::mem::take(&mut t.failed)).unwrap_or_default()
        }

        fn now_ms(&self) -> f64 {
            js_sys::Date::now()
        }
    }

    pub fn install(ctx: egui::Context) -> Drafts {
        let inbox = DraftInbox::default();
        let queue = inbox.clone();
        // Only queued here: the document opens on the next frame, never inside the callback.
        let repaint = ctx.clone();
        let restore =
            Closure::<dyn FnMut(JsValue, JsValue, JsValue, JsValue)>::new(move |key: JsValue, name: JsValue, format: JsValue, bytes: JsValue| {
                let bytes = if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
                    array.to_vec()
                } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
                    js_sys::Uint8Array::new(&bytes).to_vec()
                } else {
                    return;
                };
                let name = name.as_string().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "document.pdf".to_string());
                let restore = DraftRestore { key: key.as_string().unwrap_or_default(), name, format: format.as_string().unwrap_or_default(), bytes };
                if let Ok(mut q) = queue.lock() {
                    q.push(restore);
                }
                ctx.request_repaint();
            })
            .into_js_value();
        let sink = WebSink { restore, tracking: Default::default(), ctx: repaint };
        let drafts = Drafts::new(Box::new(sink.clone()), inbox);
        flush_when_hidden(drafts.flush_cell(), sink);
        drafts
    }

    /// The tab may be discarded or closed without another frame: put what's pending now.
    fn flush_when_hidden(cell: FlushCell, sink: WebSink) {
        let Some(window) = web_sys::window() else { return };
        let Some(document) = window.document() else { return };
        let flush = move || {
            if let Ok(mut pending) = cell.try_borrow_mut() {
                pending.flush(&sink);
            }
        };
        let on_hidden = flush.clone();
        let on_visibility = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if web_sys::window().and_then(|w| w.document()).is_some_and(|d| d.visibility_state() == web_sys::VisibilityState::Hidden) {
                on_hidden();
            }
        });
        if document.add_event_listener_with_callback("visibilitychange", on_visibility.as_ref().unchecked_ref()).is_ok() {
            on_visibility.forget();
        }
        let on_pagehide = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| flush());
        if window.add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref()).is_ok() {
            on_pagehide.forget();
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key)
}

#[cfg(target_arch = "wasm32")]
async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    use eframe::wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no window")?;
    let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| format!("{e:?}"))?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a response")?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// The `postMessage` bridge to a same-origin parent page. Messages are plain objects with a
/// `type`:
///
/// - `pdfcraft:ready` (to the parent): the app is listening; send files now.
/// - `pdfcraft:open` (from the parent): `{ name, bytes }`, where `bytes` is an `ArrayBuffer` or
///   `Uint8Array` holding a PDF. It opens in a tab, like a `?file=` URL.
/// - `pdfcraft:save` (to the parent): `{ name, saveAs, bytes }` when the user saves; `bytes` is
///   a transferred `ArrayBuffer`. The parent stores the file (and reports its own errors).
/// - `pdfcraft:dirty` (to the parent): `{ dirty }` whenever unsaved work appears or goes away.
/// - `pdfcraft:failed` (to the parent): `{ error }` when the app couldn't start.
///
/// Only the parent window on the app's own origin is heard or answered.
#[cfg(target_arch = "wasm32")]
mod host {
    use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};

    #[derive(Clone)]
    pub struct Bridge {
        window: web_sys::Window,
        parent: web_sys::Window,
        origin: String,
    }

    impl Bridge {
        /// `None` when the page isn't framed (there is no parent to talk to).
        pub fn new() -> Option<Self> {
            let window = web_sys::window()?;
            let parent = window.parent().ok().flatten()?;
            if js_sys::Object::is(&parent, &window) {
                return None;
            }
            let origin = window.location().origin().ok()?;
            Some(Self { window, parent, origin })
        }

        fn post(&self, message: &js_sys::Object, transfer: Option<&JsValue>) -> Result<(), String> {
            let sent = match transfer {
                Some(t) => self.parent.post_message_with_transfer(message, &self.origin, &js_sys::Array::of1(t)),
                None => self.parent.post_message(message, &self.origin),
            };
            sent.map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
        }

        pub fn post_ready(&self) {
            if let Some(m) = message(&[("type", "pdfcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]) {
                let _ = self.post(&m, None);
            }
        }

        /// The app couldn't start (no WebGL, for example): let the parent say so.
        pub fn post_failed(&self, error: &str) {
            if let Some(m) = message(&[("type", "pdfcraft:failed".into()), ("error", error.into())]) {
                let _ = self.post(&m, None);
            }
        }

        /// Hear `pdfcraft:open` from the parent and queue the file like a `?file=` download.
        pub fn listen(&self, inbox: pdfcraft_ui_egui::Inbox, ctx: egui::Context) {
            let parent = self.parent.clone();
            let origin = self.origin.clone();
            let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                    return;
                }
                let data = event.data();
                if string(&data, "type").as_deref() != Some("pdfcraft:open") {
                    return;
                }
                let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "document.pdf".to_string());
                let Ok(bytes) = js_sys::Reflect::get(&data, &JsValue::from_str("bytes")) else { return };
                let bytes = if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
                    array.to_vec()
                } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
                    js_sys::Uint8Array::new(&bytes).to_vec()
                } else {
                    return;
                };
                if let Ok(mut q) = inbox.lock() {
                    q.push((name, bytes));
                }
                ctx.request_repaint();
            });
            if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
                // The listener lives as long as the page.
                on_message.forget();
            }
        }

        pub fn save_fn(&self) -> pdfcraft_ui_egui::HostSaveFn {
            let bridge = self.clone();
            Box::new(move |name: &str, bytes: &[u8], save_as: bool| {
                let buffer = js_sys::Uint8Array::from(bytes).buffer();
                let m =
                    message(&[("type", "pdfcraft:save".into()), ("name", name.into()), ("saveAs", save_as.into()), ("bytes", buffer.clone().into())])
                        .ok_or_else(|| "couldn't build the message".to_string())?;
                bridge.post(&m, Some(&buffer))
            })
        }

        pub fn dirty_fn(&self) -> pdfcraft_ui_egui::HostDirtyFn {
            let bridge = self.clone();
            let mut last = false;
            Box::new(move |dirty: bool| {
                if dirty != last {
                    last = dirty;
                    if let Some(m) = message(&[("type", "pdfcraft:dirty".into()), ("dirty", dirty.into())]) {
                        let _ = bridge.post(&m, None);
                    }
                }
            })
        }
    }

    fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
        let m = js_sys::Object::new();
        for (key, value) in fields {
            js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
        }
        Some(m)
    }

    fn string(data: &JsValue, key: &str) -> Option<String> {
        js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("pdfcraft-web targets wasm32: run `trunk serve` or `trunk build --release` in apps/pdfcraft-web");
}
