//! Desktop language selection and crash-resistant preference storage.
use std::path::{Path, PathBuf};

use pdfcraft_ui_egui::{PdfCraftApp, i18n};

pub fn choose(saved: Option<&str>, explicit: Option<&str>) -> &'static str {
    explicit.and_then(i18n::normalize_pref).or_else(|| saved.and_then(i18n::normalize_pref)).unwrap_or("vi")
}

pub fn read(path: Option<&Path>) -> Option<String> {
    let path = path?;
    match std::fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<String>(&json) {
            Ok(value) => i18n::normalize_pref(&value).map(str::to_owned),
            Err(error) => {
                log::warn!("reading {}: {error}", pdfcraft_ui_egui::log_safe::home_relative(path));
                None
            }
        },
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("reading {}: {error}", pdfcraft_ui_egui::log_safe::home_relative(path));
            }
            None
        }
    }
}

fn write(path: &Path, language: &str) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::other("language settings need a parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(staged.as_file_mut(), language)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub struct DesktopApp {
    app: PdfCraftApp,
    path: Option<PathBuf>,
    saved: String,
    window_fit: Option<crate::window_fit::WindowFit>,
}

impl DesktopApp {
    pub fn new(app: PdfCraftApp, path: Option<PathBuf>) -> Self {
        let saved = app.language.clone();
        Self { app, path, saved, window_fit: None }
    }

    pub fn with_window_fit(self, window_fit: crate::window_fit::WindowFit) -> Self {
        Self { window_fit: Some(window_fit), ..self }
    }

    fn save_language(&mut self) {
        if self.saved == self.app.language {
            return;
        }
        if let Some(path) = &self.path
            && let Err(error) = write(path, &self.app.language)
        {
            log::warn!("saving {}: {error}", pdfcraft_ui_egui::log_safe::home_relative(path));
        }
        self.saved.clone_from(&self.app.language);
    }
}

impl eframe::App for DesktopApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(window_fit) = &mut self.window_fit {
            window_fit.apply(ctx);
        }
        self.app.logic(ctx, frame);
        self.save_language();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        self.save_language();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_language_precedence() {
        assert_eq!(choose(None, None), "vi");
        for saved in ["en", "vi", "auto", "ja"] {
            assert_eq!(choose(Some(saved), None), saved);
            assert_eq!(choose(Some(saved), Some("fr")), "fr");
        }
        assert_eq!(choose(None, Some("auto")), "auto");
    }

    #[test]
    fn language_change_is_saved_without_exit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language.json");
        let mut app = DesktopApp::new(PdfCraftApp::new(), Some(path.clone()));
        app.app.language = "vi".into();
        app.save_language();
        assert_eq!(read(Some(&path)).as_deref(), Some("vi"));
        app.app.language = "auto".into();
        app.save_language();
        assert_eq!(read(Some(&path)).as_deref(), Some("auto"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_language_write_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = DesktopApp::new(PdfCraftApp::new(), Some(dir.path().to_owned()));
        app.app.language = "vi".into();
        app.save_language();
        assert!(dir.path().is_dir());
    }
}
