//! Shared inert HTML builder for rich clipboard consumers.

#[cfg(target_arch = "wasm32")]
pub use pdfcraft_ui_egui::clipboard_html::paragraphs;
