//! Native rich clipboard writes with a plain-text alternative.

use std::cell::RefCell;

pub fn writer(ctx: egui::Context) -> pdfcraft_ui_egui::clipboard::TextWriter {
    // Keep the owner alive: X11 serves clipboard data from this process on demand.
    let clipboard = RefCell::new(None::<arboard::Clipboard>);
    Box::new(move |text| {
        let result = (|| {
            let mut slot = clipboard.borrow_mut();
            if slot.is_none() {
                *slot = Some(arboard::Clipboard::new()?);
            }
            if let Some(clipboard) = slot.as_mut() {
                let html = pdfcraft_ui_egui::clipboard_html::paragraphs(text);
                clipboard.set_html(html.as_str(), Some(text))?;
            }
            Ok::<(), arboard::Error>(())
        })();
        if let Err(error) = result {
            log::warn!("rich clipboard write failed: {error}");
            ctx.copy_text(text.to_owned());
        }
    })
}
