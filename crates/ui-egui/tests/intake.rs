//! Files dropped in the browser tell the office shell how they ended
//! (`window.erpIntakeResult(name, size, ok)`), once each: opened (or waiting for a password),
//! failed, or joined the Combine files list. Encrypted files dropped together ask for their
//! passwords in turn.

use std::cell::RefCell;
use std::rc::Rc;

use egui_kittest::Harness;
use pdfcraft_ui_egui::{FilePurpose, PdfCraftApp};

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

fn encrypted(password: &str) -> Vec<u8> {
    use pdfcraft_engine::{Edit, Protection, Session};
    let mut s = Session::new();
    let id = s.open("encrypted.pdf", None, std::sync::Arc::new(fixture()), None).unwrap();
    s.apply(id, Edit::Protect(Protection { open_password: Some(password.into()), ..Default::default() })).unwrap();
    s.save_bytes(id).unwrap().as_ref().clone()
}

type Reports = Rc<RefCell<Vec<(String, usize, bool)>>>;

fn harness(reports: Reports) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        // Password answers open on this thread, as in the browser.
        app.run_inline = true;
        let reports = reports.clone();
        app.intake_report = Some(Box::new(move |name: &str, size: usize, ok: bool| reports.borrow_mut().push((name.to_string(), size, ok))));
        app
    });
    h.run_steps(2);
    h
}

#[test]
fn each_dropped_file_is_reported_once_opened_or_not() {
    let reports = Reports::default();
    let mut h = harness(reports.clone());
    let good = fixture();
    h.state().queue_dropped("Báo cáo.pdf", good.clone());
    h.state().queue_dropped("broken.pdf", b"not a pdf".to_vec());
    h.run_steps(3);
    assert_eq!(*reports.borrow(), [("Báo cáo.pdf".to_string(), good.len(), true), ("broken.pdf".to_string(), 9, false)]);
    assert_eq!(h.state().views.len(), 1);
    h.run_steps(3);
    assert_eq!(reports.borrow().len(), 2, "said once");
}

#[test]
fn files_dropped_on_combine_are_reported_as_they_join_the_list() {
    let reports = Reports::default();
    let mut h = harness(reports.clone());
    let good = fixture();
    let files = vec![("one.pdf".to_string(), good.clone()), ("broken.pdf".to_string(), b"not a pdf".to_vec())];
    let request = h.state().file_request(FilePurpose::Combine, files).dropped();
    h.state().requests.lock().unwrap().push(request);
    h.run_steps(3);
    // The list says why broken.pdf can't be combined; the shell's part is done either way.
    assert_eq!(*reports.borrow(), [("one.pdf".to_string(), good.len(), true), ("broken.pdf".to_string(), 9, true)]);
    // Files picked, not dropped, are not the shell's.
    let request = h.state().file_request(FilePurpose::Combine, vec![("picked.pdf".to_string(), good)]);
    h.state().requests.lock().unwrap().push(request);
    h.run_steps(3);
    assert_eq!(reports.borrow().len(), 2);
}

#[test]
fn encrypted_files_dropped_together_ask_for_their_passwords_in_turn() {
    let reports = Reports::default();
    let mut h = harness(reports.clone());
    h.state().queue_dropped("one.pdf", encrypted("first"));
    h.state().queue_dropped("two.pdf", encrypted("second"));
    h.run_steps(3);
    let prompt = |h: &Harness<'static, PdfCraftApp>| h.state().password_prompt.as_ref().map(|p| p.name.clone());
    assert_eq!(prompt(&h).as_deref(), Some("one.pdf"));
    assert_eq!(reports.borrow().iter().map(|r| (r.0.as_str(), r.2)).collect::<Vec<_>>(), [("one.pdf", true)], "waiting for its password");
    h.state_mut().submit_password(Some("first".into()));
    h.run_steps(3);
    assert_eq!(prompt(&h).as_deref(), Some("two.pdf"), "the second file's turn");
    assert_eq!(reports.borrow().len(), 2);
    h.state_mut().submit_password(Some("second".into()));
    h.run_steps(3);
    assert_eq!(prompt(&h), None);
    let app = h.state();
    let names: Vec<String> = app.views.iter().map(|v| app.session.get(v.id).unwrap().name.clone()).collect();
    assert_eq!(names, ["one.pdf", "two.pdf"]);
}
