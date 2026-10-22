//! Signed files keep their signatures (spec OF-14): a save whose unsaved edits a certification
//! does not permit (DocMDP), or that change a field a signature locks (FieldMDP), is held, and the
//! user is offered a copy instead, so the signed original is never overwritten with a breaking
//! change. A certification that allows no changes at all is said as soon as an editing tool is
//! picked, rather than at save; one that allows some (levels 2 and 3) greys the tools it forbids.
//! Any other save of a signed file with changes asks first: the signed revision stays valid, but
//! viewers show the document as changed after signing.

use egui::{Align, Layout};
use pdfcraft_engine::commands::Needs;
use pdfcraft_engine::sign::pdf::kind;
use pdfcraft_engine::{CertificationConflict, DocId, Document};

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
    target: SaveTarget,
    affected: Vec<pdfcraft_engine::SignatureAffected>,
}

/// The sentence shown wherever a level-1 certification refuses a change.
pub(crate) fn no_changes_notice() -> &'static str {
    tl!("This document is certified and does not allow changes.")
}

/// What a level-2 or level-3 certification still allows, as one sentence.
pub(crate) fn allowed_notice(level: Option<u8>) -> &'static str {
    match level {
        Some(3) => tl!("This document is certified and allows only form filling, comments and signing."),
        _ => tl!("This document is certified and allows only form filling and signing."),
    }
}

/// Why `doc`'s certification refuses a change of `what` (a [`kind`] word), if it does.
pub(crate) fn refusal(doc: &Document, what: &str) -> Option<&'static str> {
    if doc.no_changes_certification().is_some() {
        return Some(no_changes_notice());
    }
    let c = doc.restricting_certification()?;
    (!pdfcraft_engine::sign::mdp::permits(c.certify, what)).then(|| allowed_notice(c.certify))
}

/// The kind of change a tool makes, as DocMDP counts it; `None` for one that changes nothing
/// (or only signs).
fn tool_change(tool: QuickTool) -> Option<&'static str> {
    match tool {
        QuickTool::Select | QuickTool::Hand | QuickTool::MarqueeZoom | QuickTool::Snapshot | QuickTool::ColumnSelect | QuickTool::SignArea { .. } => {
            None
        }
        QuickTool::Comment(_) | QuickTool::Measure(_) | QuickTool::Fill(_) | QuickTool::Stamp(_) | QuickTool::CustomStamp(_) => Some(kind::COMMENTS),
        QuickTool::Crop => Some(kind::PAGES),
        QuickTool::EnhanceCamera | QuickTool::Field(_) | QuickTool::Redact | QuickTool::AddText | QuickTool::EditText | QuickTool::Link => {
            Some(kind::PAGE_CONTENT)
        }
    }
}

impl PdfCraftApp {
    /// Check a save of document `id` against its signatures. Gives `after` back when the save may
    /// go ahead; otherwise the save is held (the dialog asks) or was refused with a notice.
    pub(crate) fn hold_breaking_save(&mut self, id: DocId, target: SaveTarget, after: After) -> Option<After> {
        if !self.session.get(id).is_some_and(|d| d.is_signed()) {
            return Some(after);
        }
        // Fill & Sign is baked into the page as the file is written: judge what will be written.
        if !self.flatten_fill_sign_for_save(id) {
            return None;
        }
        let conflicts = self.session.certification_conflicts(id);
        let affected = self.session.signatures_affected(id);
        if conflicts.is_empty() && affected.is_empty() {
            return Some(after);
        }
        self.held_save = Some(HeldSave { doc: id, conflicts, after, target, affected });
        None
    }

    /// Whether the user confirmed writing the current changes of signed document `id`.
    pub(crate) fn signature_write_confirmed(&self, id: DocId) -> bool {
        self.session.signatures_affected(id).is_empty()
            || self.session.get(id).is_some_and(|d| self.signature_save_ack == Some((id, d.edit_generation())))
    }

    /// Whether writing document `id` to `dest` would replace its own signed file with edits that
    /// invalidate a signature in it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn overwrites_signed_original(&self, id: DocId, dest: &str) -> bool {
        let Some(path) = self.session.get(id).and_then(|d| d.path.clone()) else { return false };
        let same = path == dest || matches!((std::fs::canonicalize(&path), std::fs::canonicalize(dest)), (Ok(a), Ok(b)) if a == b);
        same && (!self.session.certification_conflicts(id).is_empty() || self.session.signatures_affected(id).iter().any(|s| s.doc_mdp == Some(1)))
    }

    /// The held save's "Save a copy…": Save As, unguarded, since the copy is a new file.
    fn save_held_as_copy(&mut self) {
        let Some(held) = self.held_save.take() else { return };
        let Some(index) = self.views.iter().position(|v| v.id == held.doc) else { return };
        self.signature_save_ack = self.session.get(held.doc).map(|d| (held.doc, d.edit_generation()));
        self.save_guarded(index, SaveTarget::As, held.after, false);
    }

    /// A certification refuses the editing tools it forbids when they are picked, not when their
    /// work would be saved: level 1 every one, levels 2 and 3 by the change they make.
    pub(crate) fn refuse_tools_on_certified(&mut self) {
        let editing =
            !matches!(self.quick_tool, QuickTool::Select | QuickTool::Hand | QuickTool::MarqueeZoom | QuickTool::Snapshot | QuickTool::ColumnSelect);
        let Some(doc) = self.active_ids().and_then(|(_, id)| self.session.get(id)) else { return };
        let why = if doc.no_changes_certification().is_some() {
            editing.then(no_changes_notice)
        } else {
            tool_change(self.quick_tool).and_then(|what| refusal(doc, what))
        };
        if let Some(why) = why {
            self.quick_tool = QuickTool::Select;
            self.notify(why.to_string());
        }
    }

    /// Why the active document's certification disables commands that need `needs`, if it does
    /// (their tooltip and notice).
    pub(crate) fn certification_refusal(&self, needs: Needs) -> Option<&'static str> {
        let what = match needs {
            Needs::Assembly => kind::PAGES,
            Needs::Annotate => kind::COMMENTS,
            Needs::Modification | Needs::Marks(_) | Needs::HasComments | Needs::HasFields | Needs::HasRedactions => kind::PAGE_CONTENT,
            _ => return None,
        };
        refusal(self.active_ids().and_then(|(_, id)| self.session.get(id))?, what)
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

/// The held save's dialog: name who signed and what would break; Save (when the signatures allow
/// it), Save a copy… or Cancel.
pub(crate) fn show(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(held) = app.held_save.as_ref() else { return };
    let mut lines: Vec<String> = held.conflicts.iter().flat_map(explain).collect();
    let confirm_allowed = held.conflicts.is_empty() && !held.affected.iter().any(|s| s.doc_mdp == Some(1));
    // A conflict already says the signature breaks; the changed-after-signing note is for the rest.
    if held.conflicts.is_empty() && !held.affected.is_empty() {
        let names = held
            .affected
            .iter()
            .map(|s| match &s.signer {
                Some(name) => format!("{name} ({})", s.field),
                None => s.field.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(crate::i18n::fmt(
            tl!("This PDF has digital signatures: {names}. Signatures remain valid for the signed revision, but viewers will show changes after signing."),
            &[("names", &names)],
        ));
        if held.affected.iter().any(|s| matches!(s.doc_mdp, Some(2 | 3))) {
            lines.push(tl!("Saving may violate the certification's allowed-changes policy.").into());
        }
        if held.affected.iter().any(|s| s.doc_mdp == Some(1)) {
            lines.push(tl!("This certification allows no changes. Use Save a copy… to save to another file.").into());
        }
    }
    let t = Tokens::get(ctx);
    let mut copy = false;
    let mut confirm = false;
    let mut cancel = false;
    let modal = widgets::show_modal(ctx, "signed_save_guard", |ui| {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("lock", 22.0, t.accent));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(if confirm_allowed {
                        tl!("Save changes after signing?")
                    } else {
                        tl!("Saving would invalidate a signature")
                    })
                    .font(theme::semibold(16.0)),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);
        for line in &lines {
            ui.add(egui::Label::new(egui::RichText::new(line).color(t.text)).wrap());
            ui.add_space(4.0);
        }
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if confirm_allowed && widgets::pill_button(ui, tl!("Save"), true).clicked() {
                confirm = true;
            }
            if widgets::pill_button(ui, tl!("Save a copy…"), true).clicked() {
                copy = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        });
    });
    if confirm {
        if let Some(held) = app.held_save.take()
            && let Some(index) = app.views.iter().position(|v| v.id == held.doc)
        {
            app.signature_save_ack = app.session.get(held.doc).map(|d| (held.doc, d.edit_generation()));
            app.save_guarded(index, held.target, held.after, false);
        }
    } else if copy {
        app.save_held_as_copy();
    } else if cancel || modal.should_close() {
        app.held_save = None;
    }
}
