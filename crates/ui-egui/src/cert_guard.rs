//! Signed files keep their signatures (spec OF-14): a save whose unsaved edits a certification
//! does not permit (DocMDP), or that change a field a signature locks (FieldMDP), is held, and the
//! user is offered a copy instead, so the signed original is never overwritten with a breaking
//! change. A certification that allows no changes at all is said as soon as an editing tool is
//! picked, rather than at save.

use egui::{Align, Layout};
use pdfcraft_engine::{CertificationConflict, DocId};

use crate::editing::SaveTarget;
use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, QuickTool, widgets};

/// What runs once a save is written (closing the tab after Save).
pub(crate) type After = Box<dyn FnOnce(&mut PdfCraftApp) + Send>;

/// A save held because it would invalidate a signature of the document's file.
pub struct HeldSave {
    pub doc: DocId,
    pub conflicts: Vec<CertificationConflict>,
    after: After,
}

/// The sentence shown wherever a level-1 certification refuses a change.
pub(crate) fn no_changes_notice() -> &'static str {
    tl!("This document is certified and does not allow changes.")
}

impl PdfCraftApp {
    /// Check a save of document `id` against its signatures. Gives `after` back when the save may
    /// go ahead; otherwise the save is held (the dialog asks) or was refused with a notice.
    pub(crate) fn hold_breaking_save(&mut self, id: DocId, after: After) -> Option<After> {
        let restricted =
            self.session.get(id).is_some_and(|d| d.signatures.iter().any(|s| s.signed && (s.certify.is_some() || s.field_lock.is_some())));
        if !restricted {
            return Some(after);
        }
        // Fill & Sign is baked into the page as the file is written: judge what will be written.
        if !self.flatten_fill_sign_for_save(id) {
            return None;
        }
        let conflicts = self.session.certification_conflicts(id);
        if conflicts.is_empty() {
            return Some(after);
        }
        self.held_save = Some(HeldSave { doc: id, conflicts, after });
        None
    }

    /// Whether writing document `id` to `dest` would replace its own signed file with edits that
    /// invalidate a signature in it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn overwrites_signed_original(&self, id: DocId, dest: &str) -> bool {
        let Some(path) = self.session.get(id).and_then(|d| d.path.clone()) else { return false };
        let same = path == dest || matches!((std::fs::canonicalize(&path), std::fs::canonicalize(dest)), (Ok(a), Ok(b)) if a == b);
        same && !self.session.certification_conflicts(id).is_empty()
    }

    /// The held save's "Save a copy…": Save As, unguarded, since the copy is a new file.
    fn save_held_as_copy(&mut self) {
        let Some(held) = self.held_save.take() else { return };
        let Some(index) = self.views.iter().position(|v| v.id == held.doc) else { return };
        self.save_guarded(index, SaveTarget::As, held.after, false);
    }

    /// A level-1 certification refuses editing tools when they are picked, not when their work
    /// would be saved.
    pub(crate) fn refuse_tools_on_certified(&mut self) {
        let editing =
            !matches!(self.quick_tool, QuickTool::Select | QuickTool::Hand | QuickTool::MarqueeZoom | QuickTool::Snapshot | QuickTool::ColumnSelect);
        let locked = self.active_ids().and_then(|(_, id)| self.session.get(id)).is_some_and(|d| d.no_changes_certification().is_some());
        if editing && locked {
            self.quick_tool = QuickTool::Select;
            self.notify(no_changes_notice().to_string());
        }
    }
}

/// What `conflict` breaks, in the UI language.
fn explain(conflict: &CertificationConflict) -> Vec<String> {
    let name = conflict.signer.clone().unwrap_or_else(|| conflict.field.clone());
    let date = conflict.date.as_deref().map(pdfcraft_engine::sign::pdf::display_date).unwrap_or_default();
    let mut lines = Vec::new();
    let changes: Vec<String> = conflict.changes().into_iter().map(crate::i18n::words).collect();
    if !changes.is_empty() {
        lines.push(crate::i18n::fmt(
            tl!(
                "This document was certified by {name} on {date} and does not allow {kind}. Saving this change would invalidate the certification signature."
            ),
            &[("name", &name), ("date", &date), ("kind", &changes.join(", "))],
        ));
    }
    let fields = conflict.locked_fields();
    if !fields.is_empty() {
        lines.push(crate::i18n::fmt(
            tl!("The field {field} is locked by the signature of {name} on {date}. Saving this change would invalidate that signature."),
            &[("field", &fields.join(", ")), ("name", &name), ("date", &date)],
        ));
    }
    lines
}

/// The held save's dialog: name who signed and what would break; Save a copy… or Cancel.
pub(crate) fn show(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(held) = app.held_save.as_ref() else { return };
    let lines: Vec<String> = held.conflicts.iter().flat_map(explain).collect();
    let t = Tokens::get(ctx);
    let mut copy = false;
    let mut cancel = false;
    let modal = widgets::show_modal(ctx, "signed_save_guard", |ui| {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("lock", 22.0, t.accent));
            ui.add(egui::Label::new(egui::RichText::new(tl!("Saving would invalidate a signature")).font(theme::semibold(16.0))).wrap());
        });
        ui.add_space(6.0);
        for line in &lines {
            ui.add(egui::Label::new(egui::RichText::new(line).color(t.text)).wrap());
            ui.add_space(4.0);
        }
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::pill_button(ui, tl!("Save a copy…"), true).clicked() {
                copy = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        });
    });
    if copy {
        app.save_held_as_copy();
    } else if cancel || modal.should_close() {
        app.held_save = None;
    }
}
