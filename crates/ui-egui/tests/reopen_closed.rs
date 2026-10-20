//! Reopen Closed Tab (Ctrl+Shift+T, File menu, a tab's menu): the last document closed comes back
//! as it was closed, unsaved changes included, and a password-protected one asks for its password
//! again rather than coming back decrypted.
//!
//! `PDFCRAFT_SHOTS=<dir>` also writes the tab menu with the entry, in Vietnamese.

mod support;

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::{CloseRequest, PdfCraftApp};

/// An `n`-page document with a proper xref table.
fn fixture(n: usize) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
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

/// `fixture(2)` behind the opening password "pw".
fn protected() -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(fixture(2))).unwrap();
    doc.set_encryption(&pdfcraft_cos::NewEncryption {
        algorithm: pdfcraft_cos::Algorithm::Aes256,
        user_password: "pw",
        owner_password: "owner",
        permissions: -1,
        encrypt_metadata: true,
        seed: [9; 32],
    })
    .unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

fn harness(language: &'static str, setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(4);
    h
}

fn rotate_active(app: &mut PdfCraftApp) {
    assert!(app.apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
}

fn pages_rotated(app: &PdfCraftApp, view: usize) -> u16 {
    app.session.get(app.views[view].id).unwrap().info.pages[0].rotation
}

fn shot(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        h.run_steps(4);
        h.render().unwrap().save(format!("{dir}/{name}.png")).unwrap();
    }
}

#[test]
fn a_document_closed_without_saving_comes_back_with_its_changes() {
    let mut h = harness("en", |app| {
        app.open_bytes("Report.pdf", None, fixture(2)).unwrap();
        app.open_bytes("Other.pdf", None, fixture(1)).unwrap();
    });
    assert!(!h.state().can_reopen_closed());
    h.state_mut().active = Some(0);
    rotate_active(h.state_mut());
    let edited = h.state().session.get(h.state().views[0].id).unwrap().bytes.clone();
    h.state_mut().request_close_tab(0);
    h.run_steps(3);
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert_eq!(h.state().views.len(), 1);
    assert!(h.state().can_reopen_closed());

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.views.len(), 2, "a new tab");
    let doc = app.session.get(app.views[1].id).unwrap();
    assert_eq!(doc.name, "Report.pdf");
    assert!(doc.dirty, "still unsaved, as it was closed");
    assert_eq!(*doc.bytes, *edited, "with the changes it had");
    assert_eq!(pages_rotated(app, 1), 90);
    assert_eq!(app.active, Some(1));
    assert!(!app.can_reopen_closed(), "nothing more to reopen");
}

#[test]
fn reopen_says_so_when_nothing_was_closed() {
    let mut h = harness("en", |app| app.open_bytes("Report.pdf", None, fixture(1)).unwrap());
    assert!(!h.state_mut().execute("file.reopen_closed"));
    assert_eq!(h.state().toast.as_ref().map(|t| t.0.as_str()), Some("No closed document to reopen."));
    // A saved document comes back saved.
    h.state_mut().close_tab(0);
    assert!(h.state_mut().execute("file.reopen_closed"));
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!((doc.name.as_str(), doc.dirty), ("Report.pdf", false));
}

#[test]
fn every_document_closed_by_dont_save_all_can_come_back() {
    let mut h = harness("en", |app| {
        for name in ["a.pdf", "b.pdf"] {
            app.open_bytes(name, None, fixture(1)).unwrap();
            rotate_active(app);
        }
    });
    h.state_mut().close_request = Some(CloseRequest::All);
    h.run_steps(3);
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert!(h.state().views.is_empty());
    // They close from the last tab back, so reopening brings them back in their tab order.
    for expected in ["a.pdf", "b.pdf"] {
        assert!(h.state_mut().reopen_closed());
        let app = h.state();
        let doc = app.session.get(app.views.last().unwrap().id).unwrap();
        assert_eq!((doc.name.as_str(), doc.dirty), (expected, true));
    }
}

#[test]
fn a_protected_document_reopens_behind_its_password() {
    let mut h = harness("en", |app| app.open_bytes("Secret.pdf", None, protected()).unwrap());
    h.state_mut().submit_password(Some("pw".into()));
    support::wait_until(&mut h, "the protected document", |app| app.views.len() == 1 && app.password_prompt.is_none());
    rotate_active(h.state_mut());
    h.state_mut().request_close_tab(0);
    h.run_steps(3);
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert!(h.state().views.is_empty());

    assert!(h.state_mut().reopen_closed());
    h.run_steps(2);
    let prompt = h.state().password_prompt.as_ref().expect("asks for the password again");
    assert_eq!(prompt.name, "Secret.pdf");
    assert!(prompt.bytes.windows(8).any(|w| w == b"/Encrypt"), "what was kept is the encrypted file");
    assert!(!h.state().can_reopen_closed(), "one reopen at a time");
    h.state_mut().submit_password(Some("pw".into()));
    support::wait_until(&mut h, "the reopened document", |app| app.views.len() == 1 && app.password_prompt.is_none());
    h.run_steps(2);
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!((doc.name.as_str(), doc.dirty), ("Secret.pdf", true));
    assert_eq!(pages_rotated(app, 0), 90);
    assert!(doc.security_summary().is_some(), "still protected");
}

#[test]
fn cancelling_the_password_keeps_the_document_for_later() {
    let mut h = harness("en", |app| app.open_bytes("Secret.pdf", None, protected()).unwrap());
    h.state_mut().submit_password(Some("pw".into()));
    support::wait_until(&mut h, "the protected document", |app| app.views.len() == 1 && app.password_prompt.is_none());
    h.state_mut().close_tab(0);
    assert!(h.state_mut().reopen_closed());
    h.run_steps(2);
    h.state_mut().submit_password(None);
    h.run_steps(2);
    assert!(h.state().views.is_empty());
    assert!(h.state().can_reopen_closed(), "still on offer");
}

/// The tab menu's entry, in Vietnamese, after a document was closed.
#[test]
fn the_tab_menu_offers_the_closed_document_in_vietnamese() {
    let mut h = harness("vi", |app| {
        app.open_bytes("Hop dong.pdf", None, fixture(2)).unwrap();
        app.open_bytes("Bao gia.pdf", None, fixture(1)).unwrap();
        app.open_bytes("Nhat ky.pdf", None, fixture(1)).unwrap();
    });
    h.state_mut().close_tab(2);
    h.run_steps(4);
    h.get_by_role_and_label(Role::Tab, "Hop dong.pdf").click_secondary();
    h.run_steps(3);
    let entry = h.get_by_label("Mở lại tài liệu vừa đóng");
    assert!(!entry.accesskit_node().is_disabled());
    shot(&mut h, "reopen-entry-1366-vi");
    if std::env::var_os("PDFCRAFT_SHOTS").is_some() {
        h.set_size(egui::vec2(390.0, 844.0));
        h.run_steps(6);
        h.key_press(Key::Escape);
        h.run_steps(3);
        if let Some(tab) = h.query_by_role_and_label(Role::Tab, "Hop dong.pdf") {
            tab.click_secondary();
        }
        h.run_steps(3);
        shot(&mut h, "reopen-entry-390-vi");
    }
}
