//! Text still held in an inline editor (a form field, Edit text, Add text, Fill & Sign, a comment
//! being written or edited) committed to its document as the editor's own commit would, for a
//! caller that must not leave it behind: an online document switched to view or comment mode.

use pdfcraft_engine::{DocId, Edit};

use crate::PdfCraftApp;

impl PdfCraftApp {
    /// Commit every inline editor of document `id`, each as one undo step. What can't be committed
    /// (a value its field refuses, text the fonts can't draw) stays in its editor, with the reason.
    pub(crate) fn commit_inline_edits(&mut self, id: DocId) {
        let Some(i) = self.views.iter().position(|v| v.id == id) else { return };
        let active = self.active;
        // Edits apply to the active tab; the user's tab comes back afterwards.
        self.active = Some(i);
        self.commit_typing_in(id);
        let added = self.session.get(id).map(|d| d.added.clone()).unwrap_or_default();
        let prefs = self.comment_prefs.clone();
        let view = &mut self.views[i];
        let mut edits: Vec<Edit> = Vec::new();
        edits.extend(view.line_editor.take().and_then(crate::edit_text_ui::applied));
        edits.extend(crate::content_ui::commit_draft(view, &added));
        edits.extend(crate::comments::post_composer(view, &prefs));
        edits.extend(view.comments.editing.take().map(|(page, index, text)| Edit::SetAnnotationContents { page, index, text }));
        for edit in edits {
            self.apply_edit(edit);
        }
        self.active = active;
    }
}
