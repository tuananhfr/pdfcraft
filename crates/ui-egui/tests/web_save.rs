//! Headless UI tests for saving on the web where the browser has no save picker (Firefox,
//! Safari): Save asks for the download's file name in an in-app prompt first.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::web_save::{BrowserSave, FileId, SaveDone, SaveOutcome, WebSaves};

/// A one-page document with a proper xref table.
fn fixture() -> Vec<u8> {
    let objs =
        ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>", "<< /Type /Page /Parent 2 0 R >>"];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A browser without `showSaveFilePicker`: only downloads, recorded by name.
#[derive(Clone, Default)]
struct DownloadOnly(Rc<RefCell<Vec<String>>>);

impl BrowserSave for DownloadOnly {
    fn has_picker(&self) -> bool {
        false
    }
    fn pick_and_write(&self, _: &str, _: &[u8], done: SaveDone) {
        done(SaveOutcome::Failed("no picker".into()));
    }
    fn write_to(&self, _: FileId, _: &str, _: &[u8], done: SaveDone) {
        done(SaveOutcome::Failed("no picker".into()));
    }
    fn download(&self, name: &str, _: &[u8]) -> Result<(), String> {
        self.0.borrow_mut().push(name.to_string());
        Ok(())
    }
}

fn harness(browser: DownloadOnly) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, fixture()).expect("fixture opens");
        app.web_save = Some(WebSaves::new(Box::new(browser)));
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    h.run_steps(2);
    h
}

fn dirty(h: &Harness<'static, PdfCraftApp>) -> bool {
    let app = h.state();
    app.session.get(app.views[0].id).is_some_and(|d| d.dirty)
}

fn name_field<'a>(h: &'a Harness<'static, PdfCraftApp>, value: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_value(value).next().unwrap_or_else(|| panic!("no file name field showing {value:?}"))
}

#[test]
fn ctrl_s_on_a_never_saved_document_asks_for_the_name_then_downloads() {
    let browser = DownloadOnly::default();
    let mut h = harness(browser.clone());
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    name_field(&h, "doc.pdf"); // prefilled with the current name
    assert!(browser.0.borrow().is_empty(), "nothing downloads before the name is confirmed");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    name_field(&h, "doc.pdf").type_text("Minutes");
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(*browser.0.borrow(), ["Minutes.pdf"], ".pdf is kept");
    assert!(!dirty(&h));
    assert!(h.state().web_save.as_ref().unwrap().prompt().is_none());
    h.get_by_label("Minutes.pdf"); // the tab takes the new name
    assert_eq!(h.state().window_title, "Minutes.pdf — PdfCraft");
}

#[test]
fn cancelling_the_name_prompt_downloads_nothing_and_stays_unsaved() {
    let browser = DownloadOnly::default();
    let mut h = harness(browser.clone());
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    h.run_steps(3);
    name_field(&h, "doc.pdf");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(browser.0.borrow().is_empty());
    assert!(dirty(&h), "the document still shows unsaved changes");
    assert!(h.state().web_save.as_ref().unwrap().prompt().is_none());
}
