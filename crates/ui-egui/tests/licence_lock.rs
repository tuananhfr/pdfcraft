//! The desktop's licence lock through a fake account hook: while the hook names a lock, every open
//! document is view-only with the lock's reason, yet opens, saves, exports and prints; lifting it
//! restores editing with the undo history; a hook without a lock (the web's) never locks.
//!
//! `PDFCRAFT_ACCOUNT_SHOTS=<dir>` also saves screenshots of the locked states.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{DocId, Edit};
use pdfcraft_ui_egui::account::{AccountHook, AccountSnapshot, AccountView, EditLock, LicenceView, SignedInView};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp, PropsTab};

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

/// The desktop adapter as the UI sees it: a lock the test sets, and the calls the UI makes.
#[derive(Default)]
struct Licensed {
    snapshot: Mutex<AccountSnapshot>,
    lock: Mutex<Option<EditLock>>,
    calls: Mutex<Vec<String>>,
}

impl Licensed {
    fn locked(lock: EditLock) -> Arc<Self> {
        let hook = Arc::new(Self::default());
        hook.set_lock(Some(lock));
        hook
    }
    fn set_lock(&self, lock: Option<EditLock>) {
        *self.lock.lock().unwrap() = lock;
    }
    fn set_snapshot(&self, snapshot: AccountSnapshot) {
        *self.snapshot.lock().unwrap() = snapshot;
    }
    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.to_owned());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl AccountHook for Licensed {
    fn snapshot(&self) -> AccountSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    fn sign_in(&self) {
        self.record("sign_in");
    }
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {
        self.record("retry_activation");
    }
    fn allowed(&self, _feature: &str) -> bool {
        true
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
    fn gates_editing(&self) -> bool {
        true
    }
    fn edit_lock(&self) -> Option<EditLock> {
        *self.lock.lock().unwrap()
    }
    fn edit_refused(&self) {
        self.record("edit_refused");
    }
}

/// The web's adapter as far as the lock goes: it keeps every default.
struct Ungated;

impl AccountHook for Ungated {
    fn snapshot(&self) -> AccountSnapshot {
        AccountSnapshot::default()
    }
    fn sign_in(&self) {}
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {}
    fn allowed(&self, feature: &str) -> bool {
        !pdfcraft_ui_egui::account::is_pro_feature(feature)
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
}

fn harness(hook: Arc<dyn AccountHook>, path: Option<String>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1366.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.account_hook = Some(hook);
        app.open_bytes("Hop dong.pdf", path, fixture()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn doc(h: &Harness<'static, PdfCraftApp>) -> DocId {
    h.state().active_ids().unwrap().1
}

fn view_only(h: &Harness<'static, PdfCraftApp>) -> bool {
    h.state().session.get(doc(h)).unwrap().limits().view_only
}

fn rotate(h: &mut Harness<'static, PdfCraftApp>) -> bool {
    let ok = h.state_mut().apply_edit(Edit::RotatePages { pages: vec![0], degrees: 90 });
    h.run_steps(2);
    ok
}

fn toast(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    h.state().toast.as_ref().map(|(text, _)| text.clone())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-licence-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The lowest of several same-named buttons: the document's notice, under the tab strip's.
fn click_lowest(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    h.query_all_by_label(label).max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).unwrap().click();
}

fn shot(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    if let Ok(dir) = std::env::var("PDFCRAFT_ACCOUNT_SHOTS") {
        h.run_steps(4);
        h.render().unwrap().save(format!("{dir}/{name}.png")).unwrap();
    }
}

#[test]
fn a_locked_document_refuses_edits_but_saves_exports_and_prints() {
    let dir = scratch("save");
    let path = dir.join("Hop dong.pdf");
    std::fs::write(&path, fixture()).unwrap();
    let hook = Arc::new(Licensed::default());
    let mut h = harness(hook.clone(), Some(path.to_string_lossy().into_owned()));
    assert!(!view_only(&h));
    // An edit made while the licence was valid, then the lease runs out.
    assert!(rotate(&mut h));
    hook.set_lock(Some(EditLock::RenewOffline));
    h.run_steps(2);
    assert!(view_only(&h), "the lock puts the document in view-only mode");
    assert!(h.state().session.get(doc(&h)).unwrap().dirty, "the earlier change is kept");
    h.get_by_label("Connect to the internet to renew your license and edit.");

    assert!(!rotate(&mut h), "the engine refuses the edit");
    assert_eq!(toast(&h).as_deref(), Some("Connect to the internet to renew your license and edit."));
    assert!(hook.calls().contains(&"edit_refused".to_owned()), "the refusal asks the host to sync");
    assert!(!h.state_mut().execute("edit.undo"), "undo changes the document too");

    assert!(h.state_mut().execute("file.save"), "Save stays offered");
    h.run_steps(2);
    assert!(!h.state().session.get(doc(&h)).unwrap().dirty, "the change made before the lock is saved");
    let saved = std::fs::read(&path).unwrap();
    assert!(saved.len() > fixture().len(), "the file holds the incremental update");

    let exports = scratch("export");
    h.state_mut().export_dir_override = Some(exports.to_string_lossy().into_owned());
    assert!(h.state_mut().execute("export.text"));
    h.run_steps(2);
    h.get_by_label("Export").click();
    h.run_steps(4);
    assert!(exports.join("Hop dong.txt").exists(), "export still works");
    assert!(h.state_mut().execute("print.dialog"), "print still works");
    h.state_mut().dialog = None;
    h.run_steps(2);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&exports);
}

#[test]
fn a_new_lease_unlocks_without_reopening_and_keeps_the_undo_history() {
    let hook = Arc::new(Licensed::default());
    let mut h = harness(hook.clone(), None);
    assert!(rotate(&mut h));
    hook.set_lock(Some(EditLock::SignIn));
    h.run_steps(2);
    assert!(view_only(&h));
    let id = doc(&h);
    hook.set_lock(None);
    h.run_steps(2);
    assert!(!view_only(&h), "editable again");
    assert_eq!(doc(&h), id, "the same document, not reopened");
    assert!(h.state().session.get(id).unwrap().can_undo().is_some(), "the undo history survived the lock");
    assert!(rotate(&mut h));
}

#[test]
fn documents_opened_while_locked_are_locked_too() {
    let hook = Licensed::locked(EditLock::ChooseOrg);
    let mut h = harness(hook, None);
    assert!(view_only(&h));
    h.state_mut().open_bytes("Second.pdf", None, fixture()).unwrap();
    h.run_steps(1);
    assert!(view_only(&h), "locked before it is drawn");
    assert!(!rotate(&mut h));
    assert_eq!(toast(&h).as_deref(), Some("Choose an organization to edit."));
}

#[test]
fn the_notice_signs_in_or_opens_the_account_dialog() {
    let hook = Licensed::locked(EditLock::SignIn);
    let mut h = harness(hook.clone(), None);
    h.get_by_label("Sign in to PI ID to edit.");
    shot(&mut h, "licence-lock-sign-in-en");
    click_lowest(&mut h, "Sign in");
    h.run_steps(3);
    assert_eq!(hook.calls(), ["sign_in"]);
    assert_eq!(h.state().dialog, Some(Dialog::Account));
    h.state_mut().dialog = None;

    hook.set_lock(Some(EditLock::NoPlan));
    h.run_steps(3);
    h.get_by_label("This organization has no Office plan for editing.");
    click_lowest(&mut h, "Account");
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::Account));
    assert_eq!(hook.calls(), ["sign_in"], "only the sign-in reason signs in");
}

fn signed_in(licence: LicenceView) -> AccountSnapshot {
    AccountSnapshot {
        view: AccountView::SignedIn(SignedInView {
            name: "Nguyễn Văn An".into(),
            email: None,
            orgs: Vec::new(),
            active_org: Some("o1".into()),
            licence,
            warning: None,
            online: true,
            session_only: false,
            reauth_required: false,
        }),
        busy: false,
        error: None,
    }
}

#[test]
fn the_account_dialog_says_why_and_offers_retry_where_it_helps() {
    let hook = Licensed::locked(EditLock::SignIn);
    let mut h = harness(hook.clone(), None);
    h.state_mut().show_account();
    h.run_steps(3);
    assert!(h.query_by_label("Signing in is optional: every feature works without an account.").is_none(), "not on a gated desktop");
    assert!(h.query_all_by_label("Sign in to PI ID to edit.").count() >= 1);

    hook.set_snapshot(signed_in(LicenceView::Free));
    hook.set_lock(Some(EditLock::NoPlan));
    h.run_steps(3);
    h.get_by_label("Plan: Free");
    shot(&mut h, "licence-lock-account-no-plan-en");
    h.get_by_label("Retry").click();
    h.run_steps(2);
    assert_eq!(hook.calls(), ["retry_activation"]);

    hook.set_snapshot(signed_in(LicenceView::DeviceRevoked));
    hook.set_lock(Some(EditLock::DeviceRevoked));
    h.run_steps(3);
    assert!(h.query_by_label("Retry").is_none(), "Retry can't help a revoked device");
    assert!(h.query_by_label("This device was removed from your account. Free features keep working.").is_none(), "one message per fact");
}

#[test]
fn a_dialog_open_when_the_lock_comes_finishes_first() {
    let hook = Arc::new(Licensed::default());
    let mut h = harness(hook.clone(), None);
    h.state_mut().dialog = Some(Dialog::Properties(PropsTab::Description));
    h.run_steps(2);
    hook.set_lock(Some(EditLock::RenewFailed));
    h.run_steps(2);
    assert!(!view_only(&h), "the dialog's change can still be applied");
    h.state_mut().dialog = None;
    h.run_steps(2);
    assert!(view_only(&h), "locked once the dialog closes");
}

#[test]
fn a_hook_without_a_lock_never_locks_like_the_web() {
    let mut h = harness(Arc::new(Ungated), None);
    assert!(!view_only(&h));
    assert!(rotate(&mut h));
    h.get_by_label("Home").click();
    h.run_steps(2);
    h.get_by_label_contains("Signing in with PI ID is optional");

    let mut h = harness(Licensed::locked(EditLock::SignIn), None);
    h.get_by_label("Home").click();
    h.run_steps(2);
    h.get_by_label_contains("Editing needs a PI ID license");
}
