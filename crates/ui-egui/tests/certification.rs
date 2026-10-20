//! Signed files in the real shell: a save its certification (DocMDP) or a field lock (FieldMDP)
//! does not permit is held and offered as a copy, the original's bytes untouched; a level-1
//! certification refuses editing tools when they are picked, levels 2 and 3 grey the tools they
//! forbid; a locked field takes no input; an approval signature never blocks.
//!
//! `PDFCRAFT_SHOTS=<dir>` also writes the guard dialog, the level-1 bar, a locked field's tooltip
//! and the tools a level-2 certification greys, in Vietnamese.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_engine::sign::pkcs12;
use pdfcraft_engine::{Edit, FieldValue, NewAnnotation, NoteIcon, Session, Shape, SignOptions, SignatureStatus, Style};
use pdfcraft_ui_egui::{LeftPanel, PdfCraftApp, QuickTool, RightPanel, SaveTarget};

/// One page with the text fields "Amount" and "Notes" and an empty signature field "Approval"
/// carrying the `/Lock` entries `lock`.
fn form(lock: Option<&str>) -> Vec<u8> {
    let lock = lock.map(|l| format!(" /Lock << /Type /SigFieldLock {l} >>")).unwrap_or_default();
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R 8 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R 7 0 R 8 0 R] >>"
            .into(),
        "<< /Length 48 >>\nstream\nBT /F1 14 Tf 20 250 Td (Hop dong thi cong) Tj ET\nendstream".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Amount) /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Notes) /Rect [20 170 180 190] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approval) /Rect [150 20 280 70] /P 3 0 R /F 4{lock} >>"),
    ];
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

fn digital_id() -> pkcs12::DigitalId {
    let p12 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sign/tests/data/ec-p256.p12")).unwrap();
    pkcs12::open(&p12, "test").unwrap()
}

/// `form(lock)` certified at DocMDP level `certify`, or signed with an approval signature in
/// "Approval", by the fork's own signing code.
fn signed(certify: Option<u8>, lock: Option<&str>) -> Vec<u8> {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    let id = s.open("contract.pdf", None, Arc::new(form(lock)), None).unwrap();
    let opts = SignOptions {
        page: 0,
        rect: Some([20.0, 20.0, 140.0, 60.0]),
        field: certify.is_none().then(|| "Approval".to_string()),
        certify,
        ..SignOptions::default()
    };
    s.sign(id, &digital_id(), opts).unwrap().as_ref().clone()
}

/// A fresh folder per call: tests run in parallel.
fn dir() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pdfcraft-certification-ui-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The shell with `bytes` saved as `dir/contract.pdf` and opened from there; Save As writes
/// `dir/copy.pdf`.
fn harness(dir: &Path, bytes: &[u8], language: &'static str) -> Harness<'static, PdfCraftApp> {
    let original = dir.join("contract.pdf");
    std::fs::write(&original, bytes).unwrap();
    let copy = dir.join("copy.pdf").to_string_lossy().into_owned();
    let (path, bytes) = (original.to_string_lossy().into_owned(), bytes.to_vec());
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 820.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        app.open_bytes("contract.pdf", Some(path), bytes).expect("opens");
        app.set_option("left", "closed").unwrap();
        app.save_override = Some(copy);
        app
    });
    h.run_steps(6);
    h
}

fn comment() -> Edit {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    Edit::AddAnnotation(NewAnnotation { page: 0, style: Style::default_for(&shape), shape, contents: "Đã xem".into(), author: "Giám sát".into() })
}

fn fill(name: &str, text: &str) -> Edit {
    Edit::SetFieldValue { name: name.into(), value: FieldValue::Text(text.into()) }
}

fn signature_in(path: &Path) -> pdfcraft_engine::SignatureInfo {
    let mut s = Session::new();
    let id = s.open("reopened.pdf", None, Arc::new(std::fs::read(path).unwrap()), None).unwrap();
    s.get(id).unwrap().signatures.iter().find(|x| x.signed).cloned().expect("signed")
}

fn shot(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        for _ in 0..40 {
            h.run_steps(2);
            if !h.state().render_pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.render().unwrap().save(format!("{dir}/{name}.png")).unwrap();
    }
}

#[test]
fn a_comment_on_a_form_fill_certification_is_held_and_saved_as_a_copy() {
    let dir = dir();
    let certified = signed(Some(2), None);
    let mut h = harness(&dir, &certified, "en");
    assert!(h.state_mut().apply_edit(comment()));
    assert!(!h.state_mut().save_active(SaveTarget::InPlace), "held, not written");
    h.run_steps(2);
    let held = h.state().held_save.as_ref().expect("the save is held");
    assert_eq!(held.conflicts.len(), 1);
    h.get_by_label("Saving would invalidate a signature");
    h.get_by_label_contains("does not allow comments. Saving this change would invalidate the certification signature.");
    assert_eq!(std::fs::read(dir.join("contract.pdf")).unwrap(), certified, "the certified original is untouched");

    // Cancel keeps the edit and writes nothing.
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().held_save.is_none() && h.state().session.get(h.state().views[0].id).unwrap().dirty);
    assert!(!dir.join("copy.pdf").exists());

    // Save a copy… writes a new file; the original keeps its bytes and its certification.
    assert!(!h.state_mut().save_active(SaveTarget::InPlace));
    h.run_steps(2);
    h.get_by_label("Save a copy…").click();
    h.run_steps(3);
    assert!(h.state().held_save.is_none());
    let copy = std::fs::read(dir.join("copy.pdf")).expect("the copy is written");
    assert!(copy.starts_with(&certified), "an incremental copy");
    assert_eq!(std::fs::read(dir.join("contract.pdf")).unwrap(), certified);
    assert_eq!(signature_in(&dir.join("contract.pdf")).status, SignatureStatus::Unknown, "untrusted signer, intact");
    assert_eq!(signature_in(&dir.join("copy.pdf")).status, SignatureStatus::Invalid, "the copy says what the comment did");
    let doc = h.state().session.get(h.state().views[0].id).unwrap();
    assert!(!doc.dirty && doc.path.as_deref().is_some_and(|p| p.ends_with("copy.pdf")), "the tab is the copy now");
}

#[test]
fn a_copy_may_not_be_saved_over_the_original() {
    let dir = dir();
    let certified = signed(Some(2), None);
    let mut h = harness(&dir, &certified, "en");
    assert!(h.state_mut().apply_edit(comment()));
    h.state_mut().save_override = Some(dir.join("contract.pdf").to_string_lossy().into_owned());
    assert!(!h.state_mut().save_active(SaveTarget::As));
    h.run_steps(2);
    h.get_by_label("Save a copy…").click();
    h.run_steps(3);
    let toast = h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.starts_with("Choose another file"), "{toast}");
    assert_eq!(std::fs::read(dir.join("contract.pdf")).unwrap(), certified);
}

#[test]
fn permitted_edits_save_in_place_incrementally() {
    let dir = dir();
    let certified = signed(Some(2), None);
    let mut h = harness(&dir, &certified, "en");
    h.state_mut().save_override = None;
    assert!(h.state_mut().apply_edit(fill("Amount", "1 250 000")));
    assert!(h.state_mut().save_active(SaveTarget::InPlace), "form fill is permitted");
    assert!(h.state().held_save.is_none());
    let saved = std::fs::read(dir.join("contract.pdf")).unwrap();
    assert!(saved.starts_with(&certified) && saved.len() > certified.len());
    let sig = signature_in(&dir.join("contract.pdf"));
    assert_eq!((sig.certify, sig.status), (Some(2), SignatureStatus::Unknown), "{:?}", sig.details);
}

#[test]
fn a_locked_field_is_named_in_the_guard() {
    let dir = dir();
    let signed = signed(None, Some("/Action /Include /Fields [(Amount)]"));
    let mut h = harness(&dir, &signed, "en");
    h.state_mut().save_override = None;
    assert!(h.state_mut().apply_edit(fill("Notes", "Delivered")));
    assert!(h.state_mut().save_active(SaveTarget::InPlace), "Notes is not locked");
    assert!(h.state_mut().apply_edit(fill("Amount", "990 000")));
    assert!(!h.state_mut().save_active(SaveTarget::InPlace));
    h.run_steps(2);
    h.get_by_label_contains("The field Amount is locked by the signature of");
    let before = std::fs::read(dir.join("contract.pdf")).unwrap();
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(std::fs::read(dir.join("contract.pdf")).unwrap(), before);
}

#[test]
fn no_changes_certification_shows_its_bar_and_refuses_tools() {
    let dir = dir();
    let certified = signed(Some(1), None);
    let mut h = harness(&dir, &certified, "en");
    h.get_by_label("This document is certified and does not allow changes.");
    // A tool says so when picked, and drops back to Select.
    h.state_mut().quick_tool = QuickTool::Comment(pdfcraft_ui_egui::comments::CommentTool::Note);
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    let toast = h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert_eq!(toast, "This document is certified and does not allow changes.");
    // An edit reached another way is refused with the same words.
    h.state_mut().toast = None;
    assert!(!h.state_mut().apply_edit(fill("Amount", "1")));
    assert_eq!(h.state().toast.as_ref().map(|t| t.0.as_str()), Some("This document is certified and does not allow changes."));
    assert!(!h.state().session.get(h.state().views[0].id).unwrap().dirty);
    // Save As still writes the certified file as it is.
    assert!(h.state_mut().save_active(SaveTarget::As));
    assert_eq!(std::fs::read(dir.join("copy.pdf")).unwrap(), certified);
}

#[test]
fn an_approval_signature_saves_and_the_panel_reports_the_change() {
    let dir = dir();
    let signed = signed(None, None);
    let mut h = harness(&dir, &signed, "en");
    h.state_mut().save_override = None;
    assert!(h.state_mut().apply_edit(comment()));
    assert!(h.state_mut().save_active(SaveTarget::InPlace), "an approval signature never blocks");
    assert!(std::fs::read(dir.join("contract.pdf")).unwrap().starts_with(&signed));
    h.state_mut().right = Some(RightPanel::Signatures);
    h.state_mut().sig_expanded = vec!["Approval".into()];
    h.run_steps(4);
    h.get_by_label_contains("has been modified since this signature was applied, but the changes are permitted (comments");
}

/// The guard and the level-1 bar as a Vietnamese user sees them.
#[test]
fn vietnamese_guard_and_no_changes_bar() {
    let dir = dir();
    let mut h = harness(&dir, &signed(Some(2), None), "vi");
    assert!(h.state_mut().apply_edit(comment()));
    assert!(!h.state_mut().save_active(SaveTarget::InPlace));
    h.run_steps(3);
    h.get_by_label("Lưu sẽ làm chữ ký mất hiệu lực");
    h.get_by_label_contains("và không cho phép chú thích. Lưu thay đổi này sẽ làm chữ ký chứng nhận không còn hợp lệ.");
    h.get_by_label("Lưu Bản sao…");
    h.get_by_label("Hủy");
    shot(&mut h, "guard-vi");

    let dir = self::dir();
    let mut h = harness(&dir, &signed(Some(1), None), "vi");
    h.get_by_label("Tài liệu đã được chứng nhận và không cho phép thay đổi.");
    h.state_mut().quick_tool = QuickTool::Comment(pdfcraft_ui_egui::comments::CommentTool::Note);
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    shot(&mut h, "no-changes-bar-vi");
}

fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    for _ in 0..60 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn field_centre(h: &Harness<'static, PdfCraftApp>, name: &str) -> egui::Pos2 {
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    let f = doc.form.iter().find(|f| f.name == name).unwrap();
    pdfcraft_ui_egui::forms_ui::field_screen_rect(&s.views[0], &doc.info, f, 0).expect("on screen").center()
}

fn click_field(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    let p = field_centre(h, name);
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(3);
}

fn focused(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    h.state().views[0].forms.focus.as_ref().map(|f| f.name.clone())
}

fn toast(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

fn enabled(h: &Harness<'static, PdfCraftApp>, id: &str) -> bool {
    let s = h.state();
    pdfcraft_engine::commands::is_enabled(pdfcraft_engine::commands::command(id).unwrap(), &s.session, Some(s.views[0].id))
}

fn value(h: &Harness<'static, PdfCraftApp>, name: &str) -> Vec<String> {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().form.iter().find(|f| f.name == name).unwrap().value.clone()
}

#[test]
fn a_locked_field_refuses_input_and_an_unlocked_one_accepts_it() {
    let dir = dir();
    let signed = signed(None, Some("/Action /Include /Fields [(Amount)]"));
    let mut h = harness(&dir, &signed, "en");
    settle(&mut h);
    click_field(&mut h, "Amount");
    assert_eq!(focused(&h), None, "no editor opens on the locked field");
    assert!(toast(&h).starts_with("This field is locked by the signature of "), "{}", toast(&h));
    assert!(!h.state().session.get(h.state().views[0].id).unwrap().dirty);

    click_field(&mut h, "Notes");
    assert_eq!(focused(&h).as_deref(), Some("Notes"));
    h.event(egui::Event::Text("Delivered".into()));
    h.run_steps(2);
    // Tab skips the locked field: Notes is the only one left to fill.
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    assert_eq!(value(&h, "Notes"), ["Delivered"]);
    assert_eq!(focused(&h).as_deref(), Some("Notes"));
    assert!(value(&h, "Amount").is_empty());
}

#[test]
fn all_and_exclude_locks_refuse_the_fields_they_cover() {
    let dir = dir();
    let mut h = harness(&dir, &signed(None, Some("/Action /All")), "en");
    settle(&mut h);
    for name in ["Amount", "Notes"] {
        click_field(&mut h, name);
        assert_eq!(focused(&h), None, "{name} is locked");
    }
    let dir = self::dir();
    let mut h = harness(&dir, &signed(None, Some("/Action /Exclude /Fields [(Notes)]")), "en");
    settle(&mut h);
    click_field(&mut h, "Notes");
    assert_eq!(focused(&h).as_deref(), Some("Notes"), "Exclude leaves Notes open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    click_field(&mut h, "Amount");
    assert_eq!(focused(&h), None, "and locks the rest");
}

#[test]
fn a_form_fill_certification_greys_comment_and_page_tools_but_keeps_filling_and_signing() {
    let dir = dir();
    let certified = signed(Some(2), None);
    let mut h = harness(&dir, &certified, "en");
    settle(&mut h);
    h.get_by_label_contains("Only form filling and signing are allowed.");
    for id in ["comment.note", "comment.highlight", "sign.fill.text", "page.rotate", "page.delete", "edit.edit_text", "redact.mark"] {
        assert!(!enabled(&h, id), "{id} is greyed");
    }
    for id in ["form.clear", "sign.digital", "file.save_as"] {
        assert!(enabled(&h, id), "{id} still works");
    }
    let refusal = "This document is certified and allows only form filling and signing.";
    assert!(!h.state_mut().execute("page.rotate"));
    assert_eq!(toast(&h), refusal);
    // A comment tool reached another way drops back to Select, saying why.
    h.state_mut().toast = None;
    h.state_mut().quick_tool = QuickTool::Comment(pdfcraft_ui_egui::comments::CommentTool::Note);
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    assert_eq!(toast(&h), refusal);
    // The tool pane greys the same tools.
    h.state_mut().left = LeftPanel::Tool("comment");
    h.state_mut().left_open = true;
    h.run_steps(3);
    assert!(h.get_by_label("Sticky note").accesskit_node().is_disabled());
    h.state_mut().left = LeftPanel::Tool("organize");
    h.run_steps(3);
    assert!(h.get_by_label("Rotate").accesskit_node().is_disabled());
    // Filling a field still works.
    click_field(&mut h, "Amount");
    assert_eq!(focused(&h).as_deref(), Some("Amount"));
}

#[test]
fn a_comments_certification_keeps_comment_tools() {
    let dir = dir();
    let certified = signed(Some(3), None);
    let mut h = harness(&dir, &certified, "en");
    h.get_by_label_contains("Only form filling, comments and signing are allowed.");
    assert!(enabled(&h, "comment.note") && enabled(&h, "comment.highlight"));
    assert!(!enabled(&h, "page.rotate") && !enabled(&h, "edit.edit_text"));
    let note = QuickTool::Comment(pdfcraft_ui_egui::comments::CommentTool::Note);
    h.state_mut().quick_tool = note;
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, note, "kept");
    h.state_mut().quick_tool = QuickTool::Crop;
    h.run_steps(2);
    assert_eq!(h.state().quick_tool, QuickTool::Select, "page changes are still refused");
    assert_eq!(toast(&h), "This document is certified and allows only form filling, comments and signing.");
}

/// A locked field's tooltip and the tools a level-2 certification greys, as a Vietnamese user
/// sees them, at desktop and phone width.
#[test]
fn vietnamese_locked_field_and_greyed_tools() {
    let dir = dir();
    let mut h = harness(&dir, &signed(None, Some("/Action /Include /Fields [(Amount)]")), "vi");
    settle(&mut h);
    click_field(&mut h, "Amount");
    assert!(toast(&h).starts_with("Trường này đã bị khóa bởi chữ ký của "), "{}", toast(&h));
    if let Ok(out) = std::env::var("PDFCRAFT_SHOTS") {
        for (w, hgt) in [(1366.0, 768.0), (390.0, 844.0)] {
            h.state_mut().toast = None;
            h.set_size(egui::vec2(w, hgt));
            settle(&mut h);
            let p = field_centre(&h, "Amount");
            h.hover_at(p);
            for _ in 0..60 {
                h.step();
            }
            h.render().unwrap().save(format!("{out}/locked-field-{}-vi.png", w as u32)).unwrap();
        }
    }

    let dir = self::dir();
    let mut h = harness(&dir, &signed(Some(2), None), "vi");
    h.get_by_label_contains("Chỉ cho phép điền biểu mẫu và ký.");
    h.state_mut().left = LeftPanel::Tool("comment");
    h.state_mut().left_open = true;
    settle(&mut h);
    let sticky = "Ghi chú Dán";
    assert!(h.get_by_label(sticky).accesskit_node().is_disabled());
    if let Ok(out) = std::env::var("PDFCRAFT_SHOTS") {
        for (w, hgt) in [(1366.0, 768.0), (390.0, 844.0)] {
            h.set_size(egui::vec2(w, hgt));
            settle(&mut h);
            // The tool pane at desktop width; the quick bar's comment button on a phone.
            if let Some(node) = h.query_by_label(sticky).or_else(|| h.query_by_label("Thêm Ghi chú Dán")) {
                node.hover();
            }
            for _ in 0..60 {
                h.step();
            }
            h.render().unwrap().save(format!("{out}/p2-greyed-{}-vi.png", w as u32)).unwrap();
        }
    }
}
