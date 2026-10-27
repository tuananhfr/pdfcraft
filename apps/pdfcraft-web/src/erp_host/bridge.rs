//! The app's calls into the shell: `erpHost.save`, `dirty`, `opened` and `closed`.

use eframe::wasm_bindgen::{JsCast, JsValue};
use pdfcraft_ui_egui::online::{Host, Reply, SaveReason, SaveResult};

pub struct Bridge {
    host: JsValue,
    ctx: egui::Context,
}

impl Bridge {
    pub fn new(host: JsValue, ctx: egui::Context) -> Self {
        Self { host, ctx }
    }

    fn call(&self, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let f: js_sys::Function = js_sys::Reflect::get(&self.host, &JsValue::from_str(method))?.dyn_into()?;
        let args: js_sys::Array = args.iter().collect();
        f.apply(&self.host, &args)
    }
}

impl Host for Bridge {
    fn save(&self, doc_id: &str, bytes: &[u8], revision: u64, reason: SaveReason, reply: Reply) {
        // A JS-owned copy: a view into wasm memory would go stale when memory grows.
        let bytes = js_sys::Uint8Array::from(bytes);
        let options = js_sys::Object::new();
        let fields = [("revision", JsValue::from_f64(revision as f64)), ("reason", JsValue::from_str(reason.as_str()))];
        if fields.iter().any(|(key, value)| js_sys::Reflect::set(&options, &JsValue::from_str(key), value).is_err()) {
            return reply(SaveResult::Offline);
        }
        let called = self.call("save", &[JsValue::from_str(doc_id), bytes.into(), options.into()]);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // A throw or a rejection leaves the document unsaved, as `offline` does.
            let result = match called {
                Ok(value) => match wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&value)).await {
                    Ok(answer) => super::convert::save_result(&answer),
                    Err(_) => SaveResult::Offline,
                },
                Err(_) => SaveResult::Offline,
            };
            reply(result);
            ctx.request_repaint();
        });
    }

    fn dirty(&self, doc_id: &str, dirty: bool) {
        let _ = self.call("dirty", &[JsValue::from_str(doc_id), JsValue::from_bool(dirty)]);
    }

    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>) {
        let mut args = vec![JsValue::from_str(doc_id), JsValue::from_bool(ok)];
        if let Some(reason) = reason {
            args.push(JsValue::from_str(reason));
        }
        let _ = self.call("opened", &args);
    }

    fn closed(&self, doc_id: &str) {
        let _ = self.call("closed", &[JsValue::from_str(doc_id)]);
    }

    fn now_ms(&self) -> f64 {
        js_sys::Date::now()
    }
}
