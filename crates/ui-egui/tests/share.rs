//! File ▸ Share… (web, inside the office shell): hidden without a share service, it hands over the
//! document as Save a copy writes it (a signed one after asking), and downloads it when the shell
//! declines or the click was not a pointer click.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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

/// A browser that only downloads, recorded by name.
#[derive(Clone, Default)]
struct Downloads(Rc<RefCell<Vec<String>>>);

impl BrowserSave for Downloads {
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

/// What the share service was handed: (name, bytes, mime).
type Handed = Rc<RefCell<Vec<(String, Vec<u8>, String)>>>;

/// The document, rotated (an unsaved edit), with the share service answering `takes`, or none.
fn harness(takes: Option<bool>, handed: Handed, downloads: Downloads) -> Harness<'static, PdfCraftApp> {
    harness_of(fixture(), takes, handed, downloads)
}

/// `bytes` opened and rotated, with the share service answering `takes`, or none.
fn harness_of(bytes: Vec<u8>, takes: Option<bool>, handed: Handed, downloads: Downloads) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("Báo cáo.pdf", None, bytes.clone()).expect("fixture opens");
        app.web_save = Some(WebSaves::new(Box::new(downloads)));
        if let Some(takes) = takes {
            app.share = Some(Box::new(move |bytes: &[u8], name: &str, mime: &str| {
                handed.borrow_mut().push((name.to_string(), bytes.to_vec(), mime.to_string()));
                takes
            }));
        }
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    h.run_steps(2);
    h
}

fn open_file_menu(h: &mut Harness<'static, PdfCraftApp>) {
    h.get_by_label("Menu").click();
    h.run_steps(2);
    h.get_by_label("File ⏵").hover();
    h.run_steps(3);
}

#[test]
fn share_is_hidden_without_a_share_service() {
    let mut h = harness(None, Handed::default(), Downloads::default());
    open_file_menu(&mut h);
    assert!(h.query_by_label_contains("Save as…").is_some(), "the File menu is open");
    assert!(h.query_by_label_contains("Share…").is_none());
    assert!(!h.state_mut().execute("file.share"), "nor does the command run another way");
}

#[test]
fn share_hands_over_the_document_as_save_a_copy_writes_it() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = harness(Some(true), handed.clone(), downloads.clone());
    open_file_menu(&mut h);
    h.get_by_label_contains("Share…").click();
    h.run_steps(2);
    let handed = handed.borrow();
    assert_eq!(handed.len(), 1, "shared once");
    let (name, bytes, mime) = &handed[0];
    assert_eq!((name.as_str(), mime.as_str()), ("Báo cáo.pdf", "application/pdf"));
    let mut session = pdfcraft_engine::Session::new();
    let id = session.open("shared.pdf", None, Arc::new(bytes.clone()), None).expect("the shared bytes are a PDF");
    assert_eq!(session.get(id).unwrap().info.pages[0].rotation, 90, "with the unsaved edit");
    assert!(downloads.0.borrow().is_empty());
    let app = h.state();
    assert!(app.session.get(app.views[0].id).unwrap().dirty, "sharing is an export, not a save");
}

#[test]
fn a_declined_share_downloads_the_file_instead() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = harness(Some(false), handed.clone(), downloads.clone());
    open_file_menu(&mut h);
    h.get_by_label_contains("Share…").click();
    h.run_steps(2);
    assert_eq!(handed.borrow().len(), 1);
    assert_eq!(*downloads.0.borrow(), ["Báo cáo.pdf"]);
}

/// `fixture()` with an approval signature, so any change is a change after signing.
fn signed() -> Vec<u8> {
    let p12 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sign/tests/data/ec-p256.p12")).unwrap();
    let id = pdfcraft_engine::sign::pkcs12::open(&p12, "test").unwrap();
    let mut s = pdfcraft_engine::Session::new().with_clock(|| 1_800_000_000);
    let doc = s.open("signed.pdf", None, Arc::new(fixture()), None).unwrap();
    let opts = pdfcraft_engine::SignOptions { page: 0, rect: Some([20.0, 20.0, 140.0, 60.0]), ..Default::default() };
    s.sign(doc, &id, opts).unwrap().as_ref().clone()
}

const ASKS: &str = "Share changes after signing?";

/// File ▸ Share… on the signed document: the question is up and nothing has been handed over.
fn share_signed(handed: &Handed, downloads: &Downloads) -> Harness<'static, PdfCraftApp> {
    let mut h = harness_of(signed(), Some(true), handed.clone(), downloads.clone());
    open_file_menu(&mut h);
    h.get_by_label_contains("Share…").click();
    h.run_steps(3);
    h.get_by_label(ASKS);
    assert!(handed.borrow().is_empty(), "not before the user confirms");
    h
}

#[test]
fn sharing_a_signed_file_with_changes_asks_and_its_confirm_shares() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = share_signed(&handed, &downloads);
    h.get_by_label("Share…").click();
    h.run_steps(3);
    assert!(h.query_by_label(ASKS).is_none());
    let handed = handed.borrow();
    assert_eq!(handed.len(), 1, "the confirm itself shares");
    let mut session = pdfcraft_engine::Session::new();
    let id = session.open("shared.pdf", None, Arc::new(handed[0].1.clone()), None).expect("a PDF");
    let shared = session.get(id).unwrap();
    assert_eq!(shared.info.pages[0].rotation, 90, "with the change");
    assert!(shared.signatures.iter().any(|s| s.signed), "and the signature");
    assert!(downloads.0.borrow().is_empty(), "nothing downloaded");
    let app = h.state();
    assert!(app.session.get(app.views[0].id).unwrap().dirty, "nothing saved");
}

#[test]
fn cancelling_the_signed_file_question_shares_nothing() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = share_signed(&handed, &downloads);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label(ASKS).is_none());
    assert!(h.state().held_save.is_none());
    assert!(handed.borrow().is_empty() && downloads.0.borrow().is_empty());
    let app = h.state();
    assert!(app.session.get(app.views[0].id).unwrap().dirty);
}

/// Enter on the focused `label` (or one containing it): a click, but not by the pointer the
/// share sheet needs.
fn press_enter_on(h: &mut Harness<'static, PdfCraftApp>, label: &str, exact: bool) {
    if exact {
        h.get_by_label(label).focus()
    } else {
        h.get_by_label_contains(label).focus()
    }
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn share_without_a_pointer_click_downloads_instead() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = harness(Some(true), handed.clone(), downloads.clone());
    open_file_menu(&mut h);
    press_enter_on(&mut h, "Share…", false);
    assert!(handed.borrow().is_empty(), "the share sheet would refuse it");
    assert_eq!(*downloads.0.borrow(), ["Báo cáo.pdf"]);
    // Nor from the command palette or the control channel.
    assert!(h.state_mut().execute("file.share"));
    h.run_steps(2);
    assert!(handed.borrow().is_empty());
    assert_eq!(downloads.0.borrow().len(), 2);
}

#[test]
fn confirming_the_signed_file_question_from_the_keyboard_downloads_instead() {
    let (handed, downloads) = (Handed::default(), Downloads::default());
    let mut h = share_signed(&handed, &downloads);
    press_enter_on(&mut h, "Share…", true);
    assert!(h.query_by_label(ASKS).is_none());
    assert!(handed.borrow().is_empty());
    assert_eq!(*downloads.0.borrow(), ["Báo cáo.pdf"]);
}
