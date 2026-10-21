use std::sync::OnceLock;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp, branding::Branding};

#[test]
fn desktop_branding_renders_title_welcome_and_about_without_changing_default() {
    static DESKTOP: OnceLock<Branding> = OnceLock::new();
    let desktop = DESKTOP.get_or_init(|| Branding::new("PDF – ERPCons", "ERPCons", "https://office.erpcons.vn"));
    for branding in [None, Some(desktop)] {
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_eframe(move |_| {
            let mut app = PdfCraftApp::new();
            app.branding = branding;
            app.set_option("language", "vi").unwrap();
            app
        });
        h.run_steps(4);
        let name = branding.map_or("PdfCraft", |branding| branding.product_name);
        assert_eq!(h.state().window_title, name);
        h.get_by_label(&format!("Chào mừng bạn đến với {name}"));
        if branding.is_some() {
            save_shot(&mut h, "welcome.png");
        }
        h.state_mut().dialog = Some(Dialog::About);
        h.run_steps(4);
        h.get_by_label(name);
        if branding.is_some() {
            h.get_by_label("ERPCons");
            h.get_by_label("https://office.erpcons.vn");
            save_shot(&mut h, "about.png");
        }
        h.get_by_label("ERPCons Văn phòng — dựa trên PdfCraft của nhóm ArtCraft. MIT OR Apache-2.0.");
    }
    pdfcraft_ui_egui::branding::set_current(None);
}

fn save_shot(h: &mut Harness<'_, PdfCraftApp>, name: &str) {
    if let Some(dir) = std::env::var_os("PDFCRAFT_BRANDING_SHOTS_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join(name)).unwrap();
    }
}
