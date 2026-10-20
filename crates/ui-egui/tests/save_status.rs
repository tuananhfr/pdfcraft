//! A document's tab says how its save stands: "Saving…" while the browser writes it, and "Save
//! failed", with the reason on hover, from a failed save until the next save or edit. On the web,
//! Save all's prompt says the same for each document it lists, one picker (and click) at a time.
//!
//! `PDFCRAFT_SHOTS=<dir>` also writes both states on a tab, and Save all's prompt halfway, in
//! Vietnamese.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::web_save::{BrowserSave, FileId, SaveDone, SaveOutcome, WebSaves};
use pdfcraft_ui_egui::{PdfCraftApp, SaveState, SaveTarget};

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

/// A browser with a save picker that answers when the test says.
#[derive(Clone, Default)]
struct Picker(Rc<RefCell<Vec<SaveDone>>>);

impl BrowserSave for Picker {
    fn has_picker(&self) -> bool {
        true
    }
    fn pick_and_write(&self, _: &str, _: &[u8], done: SaveDone) {
        self.0.borrow_mut().push(done);
    }
    fn write_to(&self, _: FileId, _: &str, _: &[u8], done: SaveDone) {
        self.0.borrow_mut().push(done);
    }
    fn download(&self, _: &str, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

impl Picker {
    fn answer(&self, outcome: SaveOutcome) {
        let done = self.0.borrow_mut().remove(0);
        done(outcome);
    }
}

fn harness(language: &'static str, browser: Option<Picker>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        app.open_bytes("Bien ban nghiem thu.pdf", None, fixture()).unwrap();
        app.web_save = browser.map(|b| WebSaves::new(Box::new(b)));
        app
    });
    h.run_steps(4);
    rotate(&mut h);
    h
}

fn rotate(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    h.run_steps(2);
}

fn state(h: &Harness<'static, PdfCraftApp>) -> Option<SaveState> {
    h.state().save_state(h.state().views[0].id)
}

fn tab_label(h: &Harness<'static, PdfCraftApp>) -> String {
    h.query_all_by_role(Role::Tab).filter_map(|n| n.accesskit_node().label()).find(|l| l.starts_with("Bien ban")).unwrap_or_default()
}

fn shots(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") else { return };
    for (w, hgt) in [(1366.0, 768.0), (390.0, 844.0)] {
        h.set_size(egui::vec2(w, hgt));
        h.run_steps(6);
        // The reason is on the tab's tooltip: hover it.
        if let Some(tab) = h.query_all_by_role(Role::Tab).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("Bien ban"))) {
            tab.hover();
        }
        for _ in 0..40 {
            h.step();
        }
        h.render().unwrap().save(format!("{dir}/{name}-{}-vi.png", w as u32)).unwrap();
    }
    h.set_size(egui::vec2(1366.0, 768.0));
    h.run_steps(2);
}

#[test]
fn a_failed_save_shows_on_the_tab_until_the_next_edit() {
    let mut h = harness("en", None);
    // A folder that isn't there: the write fails.
    let missing = std::env::temp_dir().join(format!("pdfcraft-no-such-folder-{}", std::process::id())).join("out.pdf");
    h.state_mut().save_override = Some(missing.to_string_lossy().into_owned());
    assert!(!h.state_mut().save_active(SaveTarget::As));
    h.run_steps(2);
    let Some(SaveState::Failed(why)) = state(&h) else { panic!("{:?}", state(&h)) };
    assert!(why.starts_with("Couldn't save"), "{why}");
    assert!(tab_label(&h).ends_with("Save failed"), "{}", tab_label(&h));
    assert!(h.state().session.get(h.state().views[0].id).unwrap().dirty);
    rotate(&mut h);
    assert_eq!(state(&h), None, "an edit ends it");
    assert!(!tab_label(&h).contains("Save failed"));
}

#[test]
fn the_tab_says_saving_while_the_browser_picker_is_open() {
    let browser = Picker::default();
    let mut h = harness("en", Some(browser.clone()));
    h.state_mut().save_active(SaveTarget::InPlace);
    h.run_steps(2);
    assert_eq!(state(&h), Some(SaveState::Saving));
    assert!(tab_label(&h).ends_with("Saving…"), "{}", tab_label(&h));
    browser.answer(SaveOutcome::Saved { name: "Bien ban nghiem thu.pdf".into(), file: Some(1) });
    h.run_steps(3);
    assert_eq!(state(&h), None);
    assert_eq!(tab_label(&h), "Bien ban nghiem thu.pdf");
}

/// Both states as a Vietnamese user sees them.
#[test]
fn vietnamese_save_states_on_the_tab() {
    let browser = Picker::default();
    let mut h = harness("vi", Some(browser.clone()));
    h.state_mut().save_active(SaveTarget::InPlace);
    h.run_steps(2);
    assert!(tab_label(&h).ends_with("Đang lưu…"), "{}", tab_label(&h));
    shots(&mut h, "saving");
    browser.answer(SaveOutcome::Failed("NotAllowedError: The request is not allowed".into()));
    h.run_steps(3);
    assert!(tab_label(&h).ends_with("Lưu không thành công"), "{}", tab_label(&h));
    shots(&mut h, "save-failed");
}

/// Web Save all in Vietnamese: the first document saved, the second's picker open after its own
/// click; cancelling that picker closes nothing still unsaved.
#[test]
fn vietnamese_web_save_all_goes_one_click_per_picker() {
    let browser = Picker::default();
    let mut h = harness("vi", Some(browser.clone()));
    for name in ["Hop dong thi cong.pdf", "Bang khoi luong.pdf"] {
        h.state_mut().open_bytes(name, None, fixture()).unwrap();
        rotate(&mut h);
    }
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Lưu tất cả").click();
    h.run_steps(3);
    h.get_by_label("— Đang lưu…");
    browser.answer(SaveOutcome::Saved { name: "Bien ban nghiem thu.pdf".into(), file: Some(1) });
    h.run_steps(3);
    h.get_by_label("— Đã lưu");
    assert_eq!(browser.0.borrow().len(), 0, "the next picker waits for a click");
    h.get_by_label("Lưu “Hop dong thi cong.pdf”").click();
    h.run_steps(3);
    assert_eq!(browser.0.borrow().len(), 1);
    h.get_by_label("— Đang lưu…");
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        for (w, hgt) in [(1366.0, 768.0), (390.0, 844.0)] {
            h.set_size(egui::vec2(w, hgt));
            h.run_steps(8);
            h.render().unwrap().save(format!("{dir}/web-save-all-{}-vi.png", w as u32)).unwrap();
        }
        h.set_size(egui::vec2(1366.0, 768.0));
    }
    browser.answer(SaveOutcome::Cancelled);
    h.run_steps(3);
    assert!(h.query_by_label("Lưu thay đổi của các tài liệu này trước khi đóng?").is_none());
    assert_eq!(h.state().views.len(), 2);
    assert_eq!(h.state().unsaved_tabs(), [0, 1]);
}
