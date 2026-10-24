//! The optional PI ID account with a fake hook: nothing without one; the Account dialog, the tab
//! strip's button, Help ▸ Account and the lease expiry notice with one. Never contacts a server.
//!
//! `PDFCRAFT_ACCOUNT_SHOTS=<dir>` also saves screenshots of the states.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::account::{AccountError, AccountHook, AccountSnapshot, AccountView, LicenceView, OrgView, SignedInView, WarningView};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

#[derive(Default)]
struct Fake {
    snapshot: Mutex<AccountSnapshot>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn with(snapshot: AccountSnapshot) -> Arc<Self> {
        Arc::new(Self { snapshot: Mutex::new(snapshot), calls: Mutex::default() })
    }
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn set(&self, snapshot: AccountSnapshot) {
        *self.snapshot.lock().unwrap() = snapshot;
    }
}

impl AccountHook for Fake {
    fn snapshot(&self) -> AccountSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    fn sign_in(&self) {
        self.record("sign_in");
    }
    fn cancel_sign_in(&self) {
        self.record("cancel_sign_in");
    }
    fn sign_out(&self) {
        self.record("sign_out");
    }
    fn select_org(&self, org_id: Option<String>) {
        self.record(format!("select_org {org_id:?}"));
    }
    fn retry_activation(&self) {
        self.record("retry_activation");
    }
    fn allowed(&self, feature: &str) -> bool {
        feature != "pdf.ocr"
    }
    fn warning_shown(&self, days: u32) {
        self.record(format!("warning_shown {days}"));
    }
    fn attach(&self, _ctx: &egui::Context) {
        self.record("attach");
    }
}

fn guest() -> AccountSnapshot {
    AccountSnapshot::default()
}

fn signed_in(licence: LicenceView) -> SignedInView {
    SignedInView {
        name: "Nguyễn Văn An".into(),
        email: Some("an@example.com".into()),
        orgs: vec![
            OrgView { id: "o1".into(), name: "Công ty A".into(), role: Some("member".into()) },
            OrgView { id: "o2".into(), name: "Công ty B".into(), role: None },
        ],
        active_org: Some("o1".into()),
        licence,
        warning: None,
        online: true,
        session_only: false,
        reauth_required: false,
    }
}

fn snapshot_of(view: SignedInView) -> AccountSnapshot {
    AccountSnapshot { view: AccountView::SignedIn(view), busy: false, error: None }
}

fn harness_sized(hook: Option<Arc<Fake>>, language: &str, width: f32) -> Harness<'static, PdfCraftApp> {
    let language = language.to_string();
    let mut h = Harness::builder().with_size(egui::vec2(width, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", &language).unwrap();
        app.account_hook = hook.map(|hook| hook as Arc<dyn AccountHook>);
        app
    });
    // Fonts install on frame 1 and apply on frame 2.
    h.run_steps(4);
    h
}

fn harness(hook: Option<Arc<Fake>>) -> Harness<'static, PdfCraftApp> {
    harness_sized(hook, "en", 1200.0)
}

fn open_dialog(h: &mut Harness<'static, PdfCraftApp>) {
    h.state_mut().show_account();
    h.run_steps(3);
}

fn shot(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    if let Ok(dir) = std::env::var("PDFCRAFT_ACCOUNT_SHOTS") {
        h.run_steps(4);
        h.render().unwrap().save(format!("{dir}/{name}.png")).unwrap();
    }
}

#[test]
fn without_a_hook_nothing_shows_and_everything_is_allowed() {
    let mut h = harness(None);
    assert!(h.query_by_label("Sign in").is_none(), "no account button");
    assert!(h.state().allowed("pdf.ocr") && h.state().allowed("anything"));
    h.state_mut().show_account();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None, "no Account dialog");
    // A dialog set from elsewhere closes again instead of showing an empty page.
    h.state_mut().dialog = Some(Dialog::Account);
    h.run_steps(3);
    assert_eq!(h.state().dialog, None);
    h.get_by_label("Menu").click();
    h.run_steps(2);
    h.get_by_label("Help ⏵").hover();
    h.run_steps(3);
    assert!(h.query_by_label("Account").is_none(), "no Help ▸ Account");
    assert!(h.query_by_label_contains("Signing in with PI ID is optional").is_none());
    h.get_by_label_contains("No telemetry, no account");
}

#[test]
fn the_guest_signs_in_from_the_tab_strip_button() {
    let fake = Fake::with(guest());
    let mut h = harness(Some(fake.clone()));
    assert_eq!(fake.calls(), ["attach"], "attached once, on the first frame");
    assert!(h.state().allowed("pdf.view") && !h.state().allowed("pdf.ocr"), "the app asks the hook");
    h.get_by_label_contains("Signing in with PI ID is optional");
    shot(&mut h, "strip-guest-en");
    h.get_by_label("Sign in").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::Account));
    h.get_by_label("Signing in is optional: every feature works without an account.");
    shot(&mut h, "account-guest-en");
    h.get_by_label("Sign in with PI ID").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_in"));

    fake.set(AccountSnapshot { view: AccountView::SigningIn, busy: false, error: None });
    h.run_steps(3);
    h.get_by_label("Continue in your browser…");
    assert!(h.query_by_label("Sign in with PI ID").is_none());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("cancel_sign_in"));

    // A cancelled sign-in is not an error.
    fake.set(AccountSnapshot { view: AccountView::Guest, busy: false, error: Some(AccountError::Cancelled) });
    h.run_steps(3);
    h.get_by_label("Sign in with PI ID");
    assert!(h.query_by_label_contains("Try again").is_none(), "no error line");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
}

#[test]
fn signed_in_switches_organization_and_signs_out_in_vietnamese() {
    let fake = Fake::with(snapshot_of(signed_in(LicenceView::Free)));
    let mut h = harness_sized(Some(fake.clone()), "vi", 1200.0);
    let button = h.get_by_label("Nguyễn Văn An");
    button.click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::Account));
    h.get_by_label("Tài khoản");
    h.get_by_label("an@example.com");
    h.get_by_label("Gói: Miễn phí");
    h.get_by_label("Đã kết nối PI ID");
    shot(&mut h, "account-signed-in-vi");
    h.get_by_label("Công ty B").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some(r#"select_org Some("o2")"#));
    h.get_by_label("Cá nhân").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("select_org None"));
    h.get_by_label("Công ty A").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("select_org None"), "the active organization is not chosen again");
    h.get_by_label("Đăng xuất").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_out"));
}

#[test]
fn licence_lines_and_retry_say_each_fact_once() {
    let fake = Fake::with(AccountSnapshot {
        view: AccountView::SignedIn(signed_in(LicenceView::DeviceCapReached)),
        busy: false,
        error: Some(AccountError::DeviceCap),
    });
    let mut h = harness(Some(fake.clone()));
    open_dialog(&mut h);
    h.get_by_label("Your account has reached its device limit, so this device is not activated.");
    assert!(h.query_by_label_contains("Remove a device in PI ID").is_none(), "the error line does not repeat the licence line");
    h.get_by_label("Retry").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("retry_activation"));

    let mut view = signed_in(LicenceView::SessionOnly);
    view.session_only = true;
    view.online = false;
    view.reauth_required = true;
    fake.set(AccountSnapshot { view: AccountView::SignedIn(view), busy: false, error: Some(AccountError::ReauthRequired) });
    h.run_steps(3);
    h.get_by_label("Signed in for this session only: this computer has no credential store.");
    assert!(h.query_by_label("You will need to sign in again the next time you open the app.").is_none());
    h.get_by_label("Offline. Your account syncs when you reconnect.");
    assert_eq!(h.query_all_by_label("Your session has expired. Sign in again.").count(), 1, "said once, not again as an error");
    h.get_by_label("Sign in again").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_in"));

    fake.set(snapshot_of(signed_in(LicenceView::Active { expires_at: 1_805_112_000, pro: false })));
    h.run_steps(3);
    h.get_by_label("Plan: Free");
    h.get_by_label("This device is activated until 3/15/2027.");
}

#[test]
fn errors_show_in_plain_language() {
    let fake = Fake::with(AccountSnapshot { view: AccountView::Guest, busy: false, error: Some(AccountError::Network) });
    let mut h = harness_sized(Some(fake.clone()), "vi", 1200.0);
    open_dialog(&mut h);
    h.get_by_label("Không kết nối được PI ID. Hãy kiểm tra mạng rồi thử lại.");
    shot(&mut h, "account-error-vi");
    fake.set(AccountSnapshot { view: AccountView::Guest, busy: false, error: Some(AccountError::RateLimited { retry_after_secs: Some(150) }) });
    h.run_steps(3);
    h.get_by_label("PI ID đang bận. Hãy thử lại sau 3 phút.");
}

fn pro_lease_with_warning(pro: bool) -> AccountSnapshot {
    let mut view = signed_in(LicenceView::Active { expires_at: 1_805_112_000, pro });
    view.warning = Some(WarningView { days: 3, expires_at: 1_805_112_000 });
    snapshot_of(view)
}

#[test]
fn the_lease_warning_shows_only_for_paid_features_and_records_its_dismissal() {
    let fake = Fake::with(pro_lease_with_warning(false));
    let mut h = harness(Some(fake.clone()));
    let text = "Paid features expire on 3/15/2027. Connect to the internet to renew them.";
    assert!(h.query_by_label(text).is_none(), "a free lease warns about nothing");

    fake.set(pro_lease_with_warning(true));
    h.run_steps(3);
    let notice = h.get_by_label(text).rect();
    let mode_bar = h.get_by_label("Find tools and commands").rect();
    assert!(notice.top() >= mode_bar.bottom(), "the notice sits under the chrome, not over it: {notice:?} {mode_bar:?}");
    shot(&mut h, "lease-warning-en");
    h.get_by_label("Dismiss").click();
    h.run_steps(3);
    assert_eq!(fake.calls().last().map(String::as_str), Some("warning_shown 3"));
    assert!(h.query_by_label(text).is_none(), "closed for this run");
}

#[test]
fn help_account_opens_the_dialog() {
    let fake = Fake::with(guest());
    let mut h = harness(Some(fake));
    h.get_by_label("Menu").click();
    h.run_steps(2);
    h.get_by_label("Help ⏵").hover();
    h.run_steps(3);
    // Menu items read "<label> <shortcut>"; this one has no shortcut.
    h.get_by(|n| n.role() == egui::accesskit::Role::Button && n.label().is_some_and(|l| l.trim() == "Account")).click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::Account));
}

/// A two-page PDF, so the tab strip has a document tab to make room for.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

#[test]
fn a_narrow_tab_strip_lets_the_account_button_go_first_and_never_overflows() {
    for language in ["en", "vi"] {
        for width in [320.0, 390.0, 480.0, 600.0, 900.0] {
            for (snapshot, document) in [(guest(), false), (guest(), true), (snapshot_of(signed_in(LicenceView::Free)), true)] {
                let signed = snapshot.view != AccountView::Guest;
                let mut h = harness_sized(Some(Fake::with(snapshot)), language, width);
                if document {
                    h.state_mut().open_bytes("Quarterly report for the board.pdf", None, FIXTURE.to_vec()).unwrap();
                    h.run_steps(6);
                }
                let label = match (signed, language) {
                    (true, _) => "Nguyễn Văn An",
                    (false, "vi") => "Đăng nhập",
                    (false, _) => "Sign in",
                };
                let strip = |h: &Harness<'static, PdfCraftApp>| -> Vec<egui::Rect> {
                    h.query_all_by(|n| n.role() == egui::accesskit::Role::Button)
                        .map(|n| n.rect())
                        .filter(|r| r.top() < 38.0 && r.height() > 0.0)
                        .collect()
                };
                let Some(button) = h.query_by_label(label).map(|n| n.rect()) else {
                    assert!(document && width < 600.0, "{language} {width}: the button gives way only when the strip is short of room");
                    continue;
                };
                assert!(button.left() >= 0.0 && button.right() <= width + 0.5, "{language} {width}: {button:?} inside the window");
                for rect in strip(&h).into_iter().filter(|r| *r != button) {
                    assert!(!rect.intersects(button.shrink(0.5)), "{language} {width}: {label} {button:?} overlaps a control at {rect:?}");
                }
                if width == 390.0 && language == "vi" && !document {
                    shot(&mut h, "strip-390-vi-guest");
                }
            }
        }
    }
}
