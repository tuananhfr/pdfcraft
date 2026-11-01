//! `regionConflict(regionId)`: after a rejoin, someone else had changed a region the user changed
//! while the page had no room. The user picks here: "Giữ bản của tôi" (the shell sends this page's
//! edit), "Dùng bản mới" (the shell drops it and applies the room's content).

use egui::{Align, Layout};

use super::{Choice, ChoiceReply};
use crate::{PdfCraftApp, theme, widgets};

/// A conflict waiting for the user's choice.
pub(crate) struct ConflictPrompt {
    pub region: String,
    pub reply: ChoiceReply,
}

impl PdfCraftApp {
    /// Answer the first conflict waiting with `choice`.
    pub(crate) fn answer_conflict(&mut self, choice: Choice) {
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        if live.conflicts.is_empty() {
            return;
        }
        let prompt = live.conflicts.remove(0);
        if choice == Choice::Mine {
            self.restore_mine(&prompt.region);
        }
        (prompt.reply)(choice);
    }

    /// Put this page's last sent version of `region` back, in case the room's was applied here
    /// meanwhile; the shell sends it to the room itself.
    fn restore_mine(&mut self, region: &str) {
        let Some(live) = self.online.as_ref().and_then(|o| o.live.as_ref()) else { return };
        let (Some(id), Some(mine)) = (live.doc, live.sent.get(region).cloned()) else { return };
        if let Ok(applied) = self.session.apply_remote(id, region, &mine) {
            self.refresh_after_remote(id, &applied);
        }
    }

    /// The first conflict waiting for the user, as a modal.
    pub(crate) fn conflict_prompt(&mut self, ctx: &egui::Context) {
        let waiting = self.online.as_ref().and_then(|o| o.live.as_ref()).is_some_and(|l| !l.conflicts.is_empty());
        if !waiting {
            return;
        }
        let mut choice = None;
        widgets::show_modal(ctx, "live-conflict", |ui| {
            ui.set_width(widgets::modal_content_width(ui.ctx(), 420.0));
            ui.add(
                egui::Label::new(egui::RichText::new(tl!("Someone else changed this part while you were offline.")).font(theme::semibold(15.0)))
                    .wrap(),
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::pill_button(ui, tl!("Keep mine"), true).clicked() {
                    choice = Some(Choice::Mine);
                }
                if widgets::pill_button(ui, tl!("Use theirs"), false).clicked() {
                    choice = Some(Choice::Theirs);
                }
            });
        });
        if let Some(choice) = choice {
            self.answer_conflict(choice);
        }
    }
}
