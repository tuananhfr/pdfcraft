//! The web fallback's "Save as" prompt: where the browser has no save picker, the download's file
//! name is asked for here (see `web_save`).

use egui::{Align, Layout};

use crate::{PdfCraftApp, widgets};

pub(crate) fn show(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(prompt) = app.web_save.as_mut().and_then(|s| s.prompt.as_mut()) else { return };
    let mut save = false;
    let mut cancel = false;
    let modal = widgets::show_modal(ctx, "web_save_name", |ui| {
        ui.set_width(420.0);
        // The menu command's label without its trailing "…", as a title.
        ui.label(egui::RichText::new(tl!("Save as…").trim_end_matches('…')).font(crate::theme::semibold(18.0)));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = ui.label(tl!("File name"));
            let field = ui.add(egui::TextEdit::singleline(&mut prompt.name).desired_width(300.0)).labelled_by(label.id);
            if std::mem::take(&mut prompt.focus) {
                field.request_focus();
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
        });
        let valid = crate::web_save::pdf_file_name(&prompt.name).is_some();
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled_ui(valid, |ui| widgets::pill_button(ui, tl!("Save"), true)).inner.clicked() {
                save = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        });
    });
    if save {
        app.confirm_save_name();
    } else if cancel || modal.should_close() {
        app.cancel_save_name();
    }
}
