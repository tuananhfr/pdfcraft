//! Native update notices use a fake service and never contact a server or run installers.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable};
use pdfcraft_ui_egui::{
    PdfCraftApp,
    updates::{CheckOutcome, Offer, PreviousResult, RELEASES_PAGE, UpdateHook},
};

struct Fake {
    page_only: bool,
    auto: AtomicBool,
    calls: AtomicUsize,
    forced: AtomicBool,
    skipped: Mutex<Option<String>>,
    downloads: AtomicUsize,
    installed: AtomicBool,
    fail_download: AtomicBool,
    wait_download: AtomicBool,
    check_error: AtomicBool,
    up_to_date: AtomicBool,
    previous: Mutex<Option<PreviousResult>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            page_only: false,
            auto: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            forced: AtomicBool::new(false),
            skipped: Mutex::new(None),
            downloads: AtomicUsize::new(0),
            installed: AtomicBool::new(false),
            fail_download: AtomicBool::new(false),
            wait_download: AtomicBool::new(false),
            check_error: AtomicBool::new(false),
            up_to_date: AtomicBool::new(false),
            previous: Mutex::new(None),
        }
    }
}

impl UpdateHook for Fake {
    fn check(&self, forced: bool) -> Result<CheckOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.forced.store(forced, Ordering::SeqCst);
        if self.check_error.load(Ordering::SeqCst) {
            return Err("test check failure".into());
        }
        if self.up_to_date.load(Ordering::SeqCst) {
            return Ok(CheckOutcome::UpToDate);
        }
        Ok(CheckOutcome::Available(Offer {
            version: "99.0.0".into(),
            notes_url: format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0"),
            download_page_only: self.page_only,
        }))
    }
    fn download(&self, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        progress(5, 10);
        while self.wait_download.load(Ordering::SeqCst) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.fail_download.load(Ordering::SeqCst) {
            return Err("test download failure".into());
        }
        if cancel.load(Ordering::Relaxed) { Err("cancelled".into()) } else { Ok(()) }
    }
    fn install_and_restart(&self) -> Result<(), String> {
        self.installed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn skip(&self, version: &str) -> Result<(), String> {
        *self.skipped.lock().unwrap() = Some(version.into());
        Ok(())
    }
    fn auto_check(&self) -> bool {
        self.auto.load(Ordering::SeqCst)
    }
    fn set_auto_check(&self, on: bool) -> Result<(), String> {
        self.auto.store(on, Ordering::SeqCst);
        Ok(())
    }
    fn take_previous_result(&self) -> Option<PreviousResult> {
        self.previous.lock().unwrap().take()
    }
}

fn harness(fake: Arc<Fake>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_hook = Some(fake);
        app
    })
}

fn wait_for(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.query_by_label_contains(label).is_some() {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.get_by_label_contains(label);
}

#[test]
fn available_at_start_later_hides_and_forced_check_restores_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "PDF – ERPCons 99.0.0 is available");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(!fake.forced.load(Ordering::SeqCst));
    h.get_by_label("Remind me later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().check_for_updates();
    wait_for(&mut h, "PDF – ERPCons 99.0.0 is available");
    assert!(fake.forced.load(Ordering::SeqCst));
}

#[test]
fn skip_passes_version_and_hides_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Skip this version");
    h.get_by_label("Skip this version").click();
    h.run_steps(3);
    assert_eq!(fake.skipped.lock().unwrap().as_deref(), Some("99.0.0"));
    assert!(h.query_by_label_contains("is available").is_none());
}

#[test]
fn download_page_only_opens_notes_without_downloading() {
    let fake = Arc::new(Fake { page_only: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Download new version");
    assert!(h.query_by_label("Update now").is_none());
    h.get_by_label("Download new version").click();
    h.run_steps(3);
    assert_eq!(h.state().last_opened_url.as_deref(), Some(format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0").as_str()));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
}

#[test]
fn download_failure_offers_retry_and_success_launches_installer() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "test download failure");
    h.get_by_label("Retry");
    assert!(!fake.installed.load(Ordering::SeqCst));
    fake.fail_download.store(false, Ordering::SeqCst);
    h.get_by_label("Retry").click();
    for _ in 0..400 {
        h.run_steps(2);
        if fake.installed.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(fake.installed.load(Ordering::SeqCst));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
}

#[test]
fn previous_results_are_shown_and_failure_retry_forces_check() {
    for (previous, label, failed) in [
        (PreviousResult::Installed { version: "1.2.3".into() }, "Updated to version 1.2.3", false),
        (PreviousResult::Failed { version: "1.2.3".into(), reason: "test reason".into() }, "Update to 1.2.3 failed: test reason", true),
    ] {
        let fake = Arc::new(Fake::default());
        fake.auto.store(false, Ordering::SeqCst);
        *fake.previous.lock().unwrap() = Some(previous);
        let mut h = harness(fake.clone());
        wait_for(&mut h, label);
        assert!(fake.previous.lock().unwrap().is_none());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        if failed {
            h.get_by_label("Retry").click();
            wait_for(&mut h, "is available");
            assert!(fake.forced.load(Ordering::SeqCst));
        }
    }
}

#[test]
fn no_hook_opens_releases_page() {
    let mut app = PdfCraftApp::new();
    app.check_for_updates();
    assert_eq!(app.last_opened_url.as_deref(), Some(RELEASES_PAGE));
}

fn one_page() -> Vec<u8> {
    let objects =
        ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>", "<< /Type /Page /Parent 2 0 R >>"];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn unsaved_document_cancel_aborts_update_without_discarding_work() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    h.state_mut().open_bytes("unsaved.pdf", None, one_page()).unwrap();
    h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 });
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    h.run_steps(3);
    h.get_by_label_contains("Save changes to");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().close_request.is_none());
    assert!(h.state().first_dirty().is_some());
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn forced_checks_report_up_to_date_and_errors_when_auto_is_off() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    fake.up_to_date.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.run_steps(3);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "PDF – ERPCons is up to date.");
    fake.check_error.store(true, Ordering::SeqCst);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "test check failure");
    h.get_by_label("Retry");
}

#[test]
fn cancel_download_keeps_app_open_and_never_installs() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    h.get_by_label("Cancel").click();
    wait_for(&mut h, "Update now");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn auto_check_checkbox_updates_the_service_preference() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.state_mut().execute("app.preferences");
    h.run_steps(3);
    h.get_by_label("Automatically check for updates").click();
    h.run_steps(3);
    assert!(fake.auto.load(Ordering::SeqCst));
}

#[test]
fn update_notice_allows_document_shortcuts() {
    let mut h = harness(Arc::new(Fake::default()));
    h.state_mut().open_bytes("doc.pdf", None, one_page()).unwrap();
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    wait_for(&mut h, "Update now");
    h.get_by_label("Select (V)").click();
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!(doc.info.pages[0].rotation, 0, "the non-modal notice does not capture document shortcuts");
    assert!(doc.can_undo().is_none());
}

#[test]
fn update_notice_screenshots_in_both_themes() {
    let Some(dir) = std::env::var_os("PDFCRAFT_UPDATE_SCREENSHOTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for theme in ["light", "dark"] {
        let mut h = harness(Arc::new(Fake::default()));
        h.state_mut().set_option("language", "vi").unwrap();
        h.state_mut().set_option("theme", theme).unwrap();
        wait_for(&mut h, "Đã có PDF – ERPCons");
        h.render().unwrap().save(dir.join(format!("update-{theme}.png"))).unwrap();
    }
}

#[test]
fn edits_made_during_download_are_guarded_before_install() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.state_mut().open_bytes("doc.pdf", None, one_page()).unwrap();
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    fake.wait_download.store(false, Ordering::SeqCst);
    wait_for(&mut h, "Save changes to");
    assert!(!fake.installed.load(Ordering::SeqCst));
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.state().first_dirty().is_some());
    assert!(h.state().close_request.is_none());
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn discarding_unsaved_changes_allows_download() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.state_mut().open_bytes("doc.pdf", None, one_page()).unwrap();
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Save changes to");
    h.get_by_label("Don't save").click();
    wait_for(&mut h, "test download failure");
    assert!(h.state().first_dirty().is_none());
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
}
