//! What the app offers on a document whose embedding limits it ([`SessionLimits`], an online
//! document of the office shell, or any document under the desktop's licence lock): opened for
//! viewing or commenting, nothing may change it; with downloading turned off, it is not saved as a
//! copy, exported, printed or copied from. The engine refuses such changes at its entry points;
//! this greys the commands and drops the tools first.

use pdfcraft_engine::catalog::ToolGroup;
use pdfcraft_engine::commands::{self, CommandSpec, Needs};
use pdfcraft_engine::{DocId, SessionLimits};

use crate::{PdfCraftApp, QuickTool};

/// The sentence shown wherever a view-only document refuses a change.
pub(crate) fn view_only_notice() -> &'static str {
    tl!("This document is open for viewing only.")
}

/// [`view_only_notice`] for a document the office shell opened for commenting, which stays
/// view-only on the web until commenting comes.
pub(crate) fn comment_only_notice() -> &'static str {
    tl!("Commenting on the web is coming later; for now this document can only be viewed.")
}

/// The sentence shown wherever a document with downloading turned off refuses to let content out.
pub(crate) fn no_extract_notice() -> &'static str {
    tl!("Downloading, printing and copying are turned off for this document.")
}

/// Commands that change the document although their needs don't say so.
const CHANGING: &[&str] = &["file.save", "sign.digital", "sign.certify", "sign.certify_invisible", "a11y.autotag"];

/// Commands that take the document's content out of the app (export commands aside).
const EXTRACTING: &[&str] = &[
    "file.save_as",
    "file.share",
    "print.dialog",
    "page.copy",
    "page.cut",
    "page.extract",
    "page.split",
    "measure.export",
    "comment.export",
    "comment.summarize",
    "form.export_data",
    "optimize.reduce",
    "optimize.advanced",
];

/// Why `limits` refuse command `spec`, if they do; `read_only` says it for a view-only document.
pub(crate) fn refusal(limits: SessionLimits, spec: &CommandSpec, read_only: &'static str) -> Option<&'static str> {
    let extracts = spec.needs == Needs::Copying || spec.id.starts_with("export.") || EXTRACTING.contains(&spec.id);
    if limits.no_extract && extracts {
        return Some(no_extract_notice());
    }
    let changes = CHANGING.contains(&spec.id)
        || matches!(
            spec.needs,
            Needs::Assembly
                | Needs::Modification
                | Needs::Annotate
                | Needs::FillForms
                | Needs::HasComments
                | Needs::HasFields
                | Needs::HasRedactions
                | Needs::Marks(_)
                | Needs::Security
                | Needs::ProtectedSecurity
                | Needs::Unlock
                | Needs::Undo
                | Needs::Redo
        );
    (limits.view_only && changes).then_some(read_only)
}

impl PdfCraftApp {
    /// The limits of the active document (none without one).
    pub(crate) fn active_limits(&self) -> SessionLimits {
        self.active_ids().and_then(|(_, id)| self.session.get(id)).map(pdfcraft_engine::Document::limits).unwrap_or_default()
    }

    /// What view-only document `id` says when it refuses a change: the office shell's mode, else
    /// the licence lock's reason.
    pub(crate) fn read_only_notice(&self, id: DocId) -> &'static str {
        if self.host_comment_only(id) {
            return comment_only_notice();
        }
        match self.licence_locked(id) {
            Some(lock) => lock.reason(),
            None => view_only_notice(),
        }
    }

    /// Say why view-only document `id` refused a change.
    pub(crate) fn refuse_read_only(&mut self, id: DocId) {
        self.notify(self.read_only_notice(id).to_string());
        self.licence_refused();
    }

    /// [`Self::refuse_read_only`] for the active document.
    pub(crate) fn refuse_active_read_only(&mut self) {
        self.notify(self.active_read_only_notice().to_string());
        self.licence_refused();
    }

    /// [`Self::read_only_notice`] of the active document.
    pub(crate) fn active_read_only_notice(&self) -> &'static str {
        self.active_ids().map_or_else(view_only_notice, |(_, id)| self.read_only_notice(id))
    }

    /// Why the active document's limits refuse command `spec` (its tooltip and notice), if they do.
    pub(crate) fn limits_refusal(&self, spec: &CommandSpec) -> Option<&'static str> {
        let mut limits = self.active_limits();
        // Save stays under the licence lock, so changes made before it came are never stranded.
        if spec.id == "file.save" && self.active_ids().is_some_and(|(_, id)| self.licence_locked(id).is_some()) {
            limits.view_only = false;
        }
        refusal(limits, spec, self.active_read_only_notice())
    }

    /// Why the active document's limits refuse tool group `g` in All tools: something in it is
    /// refused and what is left only views the document (a tool for viewing stays offered).
    pub(crate) fn tool_group_refusal(&self, g: &ToolGroup) -> Option<&'static str> {
        let limits = self.active_limits();
        if limits == SessionLimits::default() {
            return None;
        }
        let mut why = None;
        for item in g.sections.iter().flat_map(|s| s.items) {
            let Some(spec) = commands::command(item.command).filter(|spec| self.command_shown(spec)) else { continue };
            match refusal(limits, spec, self.active_read_only_notice()) {
                Some(refused) => why = why.or(Some(refused)),
                None if matches!(spec.needs, Needs::Document | Needs::Nothing) => {}
                None => return None,
            }
        }
        why
    }

    /// The active document's limits refuse a tool when it is picked, however it was picked
    /// (tool pane, quick bar, key, control channel), not when its work would be applied.
    pub(crate) fn refuse_tools_on_limited(&mut self) {
        let limits = self.active_limits();
        let why = match self.quick_tool {
            QuickTool::Select | QuickTool::Hand | QuickTool::MarqueeZoom | QuickTool::ColumnSelect => None,
            QuickTool::Snapshot => limits.no_extract.then(no_extract_notice),
            _ => limits.view_only.then(|| self.active_read_only_notice()),
        };
        if let Some(why) = why {
            self.quick_tool = QuickTool::Select;
            self.notify(why.to_string());
            if limits.view_only {
                self.licence_refused();
            }
        }
    }
}
