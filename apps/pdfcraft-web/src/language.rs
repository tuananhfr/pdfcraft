//! The interface language on the ERPCons office site: Vietnamese until the visitor picks another
//! one, remembered in one localStorage key every editor of the site reads, so a choice made in
//! one editor carries over to the others.

use pdfcraft_ui_egui::PdfCraftApp;

const KEY: &str = "erpcons-ui-language";
const DEFAULT: &str = "vi";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn stored() -> Option<String> {
    storage().and_then(|s| s.get_item(KEY).ok().flatten()).filter(|v| !v.is_empty())
}

/// Wraps the app to start in the site's language and remember the visitor's choice. The app's own
/// saved settings keep everything else; the site's key wins for the language.
pub struct SiteLanguage {
    app: PdfCraftApp,
    remembered: String,
}

impl SiteLanguage {
    pub fn new(mut app: PdfCraftApp) -> Self {
        app.language = stored().unwrap_or_else(|| DEFAULT.into());
        let remembered = app.language.clone();
        Self { app, remembered }
    }
}

impl eframe::App for SiteLanguage {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
        if self.app.language != self.remembered {
            self.remembered.clone_from(&self.app.language);
            // Storage may be off (private mode); that only costs the choice on reload.
            if let Some(s) = storage()
                && let Err(e) = s.set_item(KEY, &self.remembered)
            {
                eframe::web_sys::console::warn_1(&format!("PdfCraft: couldn't remember the interface language: {e:?}").into());
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}
