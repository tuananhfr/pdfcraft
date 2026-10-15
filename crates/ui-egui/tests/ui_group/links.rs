//! Project links: the Help menu and the About dialog open this fork's source. Nothing links to
//! ArtCraft or its Discord: the ERPCons fork drops the marks and community links (brand terms).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::links;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn harness(setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_top_bar_has_no_community_button() {
    let h = harness(|_| {});
    h.get_by_label("Keyboard shortcuts");
    assert!(h.query_by_label("Discord").is_none());
}

#[test]
fn the_home_screen_has_no_community_card() {
    let h = harness(|_| {});
    h.get_by_label("Welcome to PdfCraft");
    assert!(h.query_by_label_contains("ArtCraft").is_none());
    assert!(h.query_by_label_contains("Discord").is_none());
}

#[test]
fn about_credits_upstream_and_links_the_fork() {
    let pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF";
    // With a document open, so only the dialog's own text is on screen.
    let mut h = harness(move |app| {
        app.open_bytes("one.pdf", None, pdf.to_vec()).unwrap();
        app.dialog = Some(Dialog::About);
    });
    assert_eq!(h.query_all_by_label("ArtCraft").count(), 0, "no ArtCraft mark or wordmark");
    h.get_by_label("ERPCons Văn phòng — dựa trên PdfCraft của nhóm ArtCraft. MIT OR Apache-2.0.");
    h.get_by_label("PdfCraft on GitHub").click();
    h.run_steps(2);
    assert_eq!(h.state().last_opened_url.as_deref(), Some("https://github.com/tuananhfr/pdfcraft"));
}

#[test]
fn help_commands_open_each_link() {
    for l in links::LINKS {
        let mut h = harness(|_| {});
        assert!(h.state_mut().execute(l.command), "{}", l.command);
        assert_eq!(h.state().last_opened_url.as_deref(), Some(l.url));
        assert_eq!(pdfcraft_engine::commands::command(l.command).unwrap().menu, Some("Help"));
    }
}
