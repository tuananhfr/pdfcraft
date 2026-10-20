//! The optional ERP shell notification after closing the last document.

fn should_notify(before: usize, after: usize) -> bool {
    before == 1 && after == 0
}

pub(crate) fn document_closed(before: usize, after: usize) {
    if should_notify(before, after) {
        notify_host();
    }
}

#[cfg(target_arch = "wasm32")]
fn notify_host() {
    use wasm_bindgen::{JsCast, JsValue};

    let Some(window) = web_sys::window() else { return };
    let hook = js_sys::Reflect::get(&window, &JsValue::from_str("erpEditorClosed"));
    let Some(hook) = hook.ok().and_then(|v| v.dyn_into::<js_sys::Function>().ok()) else { return };
    if let Err(e) = hook.call0(&window) {
        log::warn!("erpEditorClosed failed: {e:?}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn notify_host() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_only_fires_when_the_last_document_closes() {
        assert!(!should_notify(2, 1));
        assert!(should_notify(1, 0));
        assert!(!should_notify(0, 0));
        assert!(!should_notify(1, 1));
    }

    #[test]
    fn reopening_allows_one_more_last_close_notification() {
        let transitions = [(2, 1), (1, 0), (0, 0), (0, 1), (1, 0), (0, 0)];
        assert_eq!(transitions.into_iter().filter(|&(before, after)| should_notify(before, after)).count(), 2);
    }
}
