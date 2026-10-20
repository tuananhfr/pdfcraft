//! Word export confirmation in the shared desktop/web shell.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{Edit, Session};
use pdfcraft_ui_egui::PdfCraftApp;

const TITLE: &str = "Xuất Word sẽ mất một số nội dung";

fn annotated_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Text /Rect [30 30 50 50] /Contents (Review) >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn output_path(case: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("pdfcraft-word-{case}-{}.docx", std::process::id()));
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    path
}

fn harness(bytes: Vec<u8>, output: &std::path::Path, size: egui::Vec2) -> Harness<'static, PdfCraftApp> {
    let output = output.to_string_lossy().into_owned();
    Harness::builder().with_size(size).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "vi").unwrap();
        app.open_bytes("doc.pdf", None, bytes.clone()).unwrap();
        app.save_override = Some(output.clone());
        app
    })
}

#[test]
fn word_export_annotation_cancel_writes_nothing() {
    let output = output_path("cancel");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    assert_eq!(h.query_all_by_label(TITLE).count(), 1);
    h.get_by_label("Chú thích và nhận xét sẽ không được giữ lại: 1");
    h.get_by_label("Bố cục có thể thay đổi.");
    assert!(!output.exists());
    h.get_by_label("Hủy").click();
    h.run_steps(4);
    assert!(h.query_by_label(TITLE).is_none());
    assert!(!output.exists());
}

#[test]
fn word_export_annotation_confirm_proceeds_once() {
    let output = output_path("confirm");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(2);
    assert_eq!(h.query_all_by_label(TITLE).count(), 1);
    assert!(!output.exists());
    h.get_by_label("Vẫn xuất").click();
    h.run_steps(4);
    assert!(h.query_by_label(TITLE).is_none());
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert!(bytes.windows(b"word/document.xml".len()).any(|part| part == b"word/document.xml"));
    std::fs::remove_file(&output).unwrap();
    h.run_steps(6);
    assert!(!output.exists(), "the confirmation must not queue a second export");
}

#[test]
fn word_export_clean_text_needs_no_confirmation() {
    let output = output_path("clean");
    let bytes = Session::new().create_from_text("Clean", "Only text").unwrap().to_vec();
    let mut h = harness(bytes, &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(4);
    assert!(h.query_by_label(TITLE).is_none());
    assert!(std::fs::read(&output).unwrap().starts_with(b"PK"));
    std::fs::remove_file(output).unwrap();
}

#[test]
fn word_export_consent_stays_with_its_document_after_a_tab_change() {
    let output = output_path("tab");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    let id = h.state().active_ids().unwrap().1;
    let expected = h.state().session.get(id).unwrap().export_office(pdfcraft_engine::compare::OfficeFormat::Docx);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    let clean = Session::new().create_from_text("Other", "A different document").unwrap().to_vec();
    h.state_mut().open_bytes("other.pdf", None, clean).unwrap();
    h.run_steps(3);
    h.get_by_label("Vẫn xuất").click();
    h.run_steps(4);
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    std::fs::remove_file(output).unwrap();
}

#[test]
fn word_export_changed_document_requires_fresh_consent() {
    let output = output_path("changed");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    let id = h.state().active_ids().unwrap().1;
    h.state_mut().session.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
    h.get_by_label("Vẫn xuất").click();
    h.run_steps(4);
    h.get_by_label(TITLE);
    assert!(!output.exists());
    h.get_by_label("Vẫn xuất").click();
    h.run_steps(4);
    assert!(output.exists());
    std::fs::remove_file(output).unwrap();
}

#[test]
fn word_export_closed_document_cancels_the_pending_request() {
    let output = output_path("closed");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    h.get_by_label(TITLE);
    let index = h.state().active_ids().unwrap().0;
    h.state_mut().close_tab(index);
    h.run_steps(4);
    assert!(h.query_by_label(TITLE).is_none());
    assert!(!output.exists());
}

#[test]
fn word_export_escape_cancels_and_other_formats_keep_their_behavior() {
    let output = output_path("escape");
    let mut h = harness(annotated_pdf(), &output, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert!(h.state_mut().execute("export.docx"));
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label(TITLE).is_none());
    assert!(!output.exists());
    assert!(h.state_mut().execute("export.html"));
    h.run_steps(3);
    assert!(h.query_by_label(TITLE).is_none());
    assert!(std::fs::read_to_string(&output).unwrap().contains("<html"));
    std::fs::remove_file(output).unwrap();
}

#[test]
fn word_export_dialog_fits_phone_tablet_and_desktop() {
    for (case, size) in [
        ("phone", egui::vec2(390.0, 844.0)),
        ("tablet", egui::vec2(768.0, 1024.0)),
        ("desktop", egui::vec2(1400.0, 900.0)),
    ] {
        let output = output_path(case);
        let mut h = harness(annotated_pdf(), &output, size);
        h.run_steps(4);
        assert!(h.state_mut().execute("export.docx"));
        h.run_steps(4);
        let card: egui::Rect = h.ctx.data(|data| data.get_temp(egui::Id::new("word_export_losses").with("card"))).unwrap();
        assert!(h.ctx.content_rect().contains_rect(card), "{case}: {card:?}");
        for label in [TITLE, "Chú thích và nhận xét sẽ không được giữ lại: 1", "Bố cục có thể thay đổi.", "Vẫn xuất", "Hủy"] {
            assert!(card.contains_rect(h.get_by_label(label).rect()), "{case}: {label}");
        }
        assert!(!h.get_by_label("Vẫn xuất").rect().intersects(h.get_by_label("Hủy").rect()), "{case}: buttons overlap");
        if let Ok(dir) = std::env::var("PDFCRAFT_SHOT_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            h.render().unwrap().save(dir.join(format!("word-export-{case}.png"))).unwrap();
        }
        h.get_by_label("Hủy").click();
        h.run_steps(3);
        assert!(!output.exists());
    }
}
