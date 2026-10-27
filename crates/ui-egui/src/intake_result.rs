//! How a file the office shell delivered by drop ended (`window.erpIntakeResult`, web only): by
//! its name and size, opened or not, and nothing else. Without the hook nothing happens.

/// Report one dropped file once the editor decided what became of it.
pub(crate) fn report(name: &str, size: usize, ok: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(window) = web_sys::window() else { return };
        let Some(hook) =
            js_sys::Reflect::get(&window, &JsValue::from_str("erpIntakeResult")).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        else {
            return;
        };
        // Fire and forget: the shell's bookkeeping never holds up opening.
        let _ = hook.call3(&window, &JsValue::from_str(name), &JsValue::from_f64(size as f64), &JsValue::from_bool(ok));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (name, size, ok);
}

/// The bytes the browser read from a dropped file of `size` bytes, or `None` after telling the
/// shell (through `report`) that it could not be read, so its entry does not wait forever.
// Only the browser reads dropped files asynchronously; native drops arrive as bytes or paths.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn read_or_report<E>(name: &str, size: usize, read: Result<Vec<u8>, E>, report: impl FnOnce(&str, usize, bool)) -> Option<Vec<u8>> {
    match read {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            report(name, size, false);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_or_report;

    #[test]
    fn an_unreadable_drop_is_reported_failed_with_its_size_and_a_read_one_waits_for_opening() {
        let mut reported = Vec::new();
        let read = read_or_report("Báo cáo.pdf", 4096, Err("NotReadableError"), |n, s, ok| reported.push((n.to_string(), s, ok)));
        assert_eq!(read, None);
        assert_eq!(reported, [("Báo cáo.pdf".to_string(), 4096, false)]);
        let read = read_or_report::<()>("Báo cáo.pdf", 3, Ok(b"%PD".to_vec()), |_, _, _| panic!("reported only once it is opened"));
        assert_eq!(read.as_deref(), Some(&b"%PD"[..]));
    }
}
