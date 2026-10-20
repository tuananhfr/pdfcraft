//! Office export and the confirmation for content Word cannot preserve.

use std::sync::Arc;

use egui::{Align, Layout};
use pdfcraft_engine::compare::OfficeFormat;
use pdfcraft_engine::export::{LossItem, export_losses};
use pdfcraft_engine::DocId;

use crate::theme;
use crate::{PdfCraftApp, widgets};

pub(crate) struct PendingWordExport {
    doc: DocId,
    bytes: Arc<Vec<u8>>,
    losses: Vec<LossItem>,
}

fn loss_label(loss: &LossItem) -> String {
    match loss {
        LossItem::FormFields(count) => crate::i18n::fmt(tl!("Form fields will not be kept: {n}"), &[("n", &count.to_string())]),
        LossItem::Annotations(count) => crate::i18n::fmt(tl!("Annotations and comments will not be kept: {n}"), &[("n", &count.to_string())]),
        LossItem::DigitalSignatures => tl!("Digital signatures and signature fields will not be kept.").into(),
        LossItem::EncryptionPermissions => tl!("PDF encryption and permissions will not be kept.").into(),
        LossItem::LayoutMayChange => tl!("The layout may change.").into(),
    }
}

pub(crate) fn show(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(pending) = app.pending_word_export.as_ref() else { return };
    if app.session.get(pending.doc).is_none() {
        app.pending_word_export = None;
        return;
    }
    let mut proceed = false;
    let mut cancel = false;
    let modal = widgets::show_modal(ctx, "word_export_losses", |ui| {
        ui.set_width((ctx.content_rect().width() - 64.0).clamp(0.0, 460.0));
        ui.add(egui::Label::new(egui::RichText::new(tl!("Exporting to Word will lose some content")).font(theme::semibold(16.0))).wrap());
        ui.add_space(8.0);
        for loss in &pending.losses {
            ui.add(egui::Label::new(loss_label(loss)).wrap());
            ui.add_space(4.0);
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::pill_button(ui, tl!("Export anyway"), true).clicked() {
                proceed = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        });
    });
    if cancel || modal.should_close() {
        app.pending_word_export = None;
    } else if proceed && let Some(pending) = app.pending_word_export.take() {
        // Consent belongs to the document state shown, even if automation changes tabs or edits it.
        if let Err(m) = pdfcraft_engine::guard(|| {
            if app.session.get(pending.doc).is_some_and(|doc| Arc::ptr_eq(&doc.bytes, &pending.bytes)) {
                app.write_office_export(pending.doc, OfficeFormat::Docx);
            } else {
                app.request_office_export(pending.doc, OfficeFormat::Docx);
            }
        }) {
            app.notify_fmt("That didn't work: an internal error stopped it ({m}). Your documents are unchanged.", &[("m", m.as_str())]);
        }
    }
}

impl PdfCraftApp {
    /// Ask where to export Word, PowerPoint, HTML or RTF (`save_override` in tests).
    pub fn export_office_dialog(&mut self, format: OfficeFormat) {
        if self.pending_word_export.is_some() || !self.commit_form_typing() {
            return;
        }
        let Some((_, id)) = self.active_ids() else { return };
        self.request_office_export(id, format);
    }

    fn request_office_export(&mut self, id: DocId, format: OfficeFormat) {
        if !self.may_copy(id) {
            return;
        }
        let Some(doc) = self.session.get(id) else { return };
        if format == OfficeFormat::Docx {
            let losses = export_losses(doc);
            if !losses.is_empty() {
                self.pending_word_export = Some(PendingWordExport { doc: id, bytes: doc.bytes.clone(), losses });
                return;
            }
        }
        self.write_office_export(id, format);
    }

    fn write_office_export(&mut self, id: DocId, format: OfficeFormat) {
        if !self.may_copy(id) {
            return;
        }
        let Some(doc) = self.session.get(id) else { return };
        let stem = doc.name.trim_end_matches(".pdf").trim_end_matches(".PDF").to_string();
        let ext = format.extension();
        let bytes = doc.export_office(format);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let write = move |app: &mut Self, path: std::path::PathBuf| match crate::editing::write_atomically(&path.to_string_lossy(), &bytes) {
                Ok(()) => app.notify_fmt("Exported to {path}", &[("path", &path.display().to_string())]),
                Err(e) => app.notify_fmt("Couldn't write {name}: {e}", &[("name", &path.display().to_string()), ("e", &e.to_string())]),
            };
            match self.save_override.clone() {
                Some(p) => write(self, p.into()),
                None => {
                    let dialog = rfd::AsyncFileDialog::new()
                        .set_title(tl!("Export").to_string())
                        .add_filter(ext.to_uppercase(), &[ext])
                        .set_file_name(format!("{stem}.{ext}"));
                    self.ask_one(crate::pickers::Ask::Save(dialog), None, write);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Err(e) = crate::editing::download(&format!("{stem}.{ext}"), &bytes) {
            self.notify_error(e);
        }
    }
}
