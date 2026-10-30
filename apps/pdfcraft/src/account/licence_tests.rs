//! The edit lock against pi-mock's Office free plan (`erpconsoffice`): what locks editing, what
//! lifts it, and that the UI's question never waits for the network.

use std::sync::atomic::{AtomicI64, AtomicU64};
use std::time::{Duration, Instant};

use erpcons_account::{Clock, LicenceState, MemorySecretStore, OFFICE_USE, PinnedKey, SignedIn};
use pi_mock::{FREE_OFFICE, Fault, OFFICE_PRODUCT, ORG_OFFICE, Scenario, Server, Tenant};

use super::tests::{attached, follow_redirect, mock_config, wait_for};
use super::*;

const T0: i64 = 1_792_828_800;
const DAY: i64 = 86_400;

struct TestClock {
    unix: AtomicI64,
    mono_ms: AtomicU64,
}

impl Clock for TestClock {
    fn unix_now(&self) -> i64 {
        self.unix.load(Ordering::SeqCst)
    }
    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.mono_ms.load(Ordering::SeqCst))
    }
}

struct Env {
    server: Option<Server>,
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
}

impl Env {
    fn new(mut scenario: Scenario) -> Self {
        scenario.now.get_or_insert(T0);
        Self {
            server: Some(Server::start(scenario)),
            dir: tempfile::tempdir().unwrap(),
            clock: Arc::new(TestClock { unix: AtomicI64::new(T0), mono_ms: AtomicU64::new(1_000_000) }),
        }
    }

    fn server(&self) -> &Server {
        self.server.as_ref().unwrap()
    }

    /// The app's adapter with a test clock, an in-memory credential store and the test browser;
    /// refused edits resync within milliseconds.
    fn hook(&self) -> DesktopAccount {
        let mut config = mock_config(self.server(), &self.dir);
        config.clock = self.clock.clone();
        let hook = DesktopAccount::new(config, Arc::new(MemorySecretStore::new()), Box::new(follow_redirect))
            .with_resync(Duration::from_millis(20), Duration::from_millis(50));
        attached(hook)
    }

    /// Moves client wall clock, client monotonic clock and platform time together.
    fn advance(&self, secs: i64) {
        self.clock.unix.fetch_add(secs, Ordering::SeqCst);
        self.clock.mono_ms.fetch_add(secs.unsigned_abs() * 1000, Ordering::SeqCst);
        let now = self.clock.unix_now();
        if let Some(server) = &self.server {
            server.update_scenario(|s| s.now = Some(now));
        }
    }
}

fn account(hook: &DesktopAccount) -> &Arc<Account> {
    hook.shared.account.get().unwrap()
}

fn licence(hook: &DesktopAccount) -> Option<LicenceState> {
    match account(hook).status() {
        Status::SignedIn(s) => Some(s.licence),
        _ => None,
    }
}

fn wait_lock(hook: &DesktopAccount, what: &str, lock: Option<EditLock>) {
    let until = Instant::now() + Duration::from_secs(30);
    while hook.edit_lock() != lock {
        assert!(Instant::now() < until, "timed out waiting for {what}: {:?}, licence {:?}", hook.edit_lock(), licence(hook));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sign_in(hook: &DesktopAccount) {
    hook.sign_in();
    wait_for(hook, "sign-in", |s| matches!(s.view, AccountView::SignedIn(_)));
}

#[test]
fn a_guest_is_locked_until_the_free_plan_lease_arrives() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    wait_for(&hook, "guest", |s| s.view == AccountView::Guest);
    assert_eq!(hook.edit_lock(), Some(EditLock::SignIn));
    assert!(hook.gates_editing());

    sign_in(&hook);
    wait_lock(&hook, "the lease of the free plan", None);
    let Some(LicenceState::Active { features, .. }) = licence(&hook) else { panic!("no lease: {:?}", licence(&hook)) };
    assert!(features.iter().any(|f| f == OFFICE_USE), "{features:?}");
    let AccountView::SignedIn(view) = hook.snapshot().view else { panic!("signed in") };
    assert_eq!(view.active_org.as_deref(), Some(ORG_OFFICE));
    assert!(matches!(view.licence, LicenceView::Active { pro: false, .. }), "the free plan is no paid plan: {:?}", view.licence);
    let activations = env.server().requests().into_iter().filter(|r| r.path.ends_with("/devices/activate")).collect::<Vec<_>>();
    assert!(!activations.is_empty() && activations.iter().all(|r| r.body.contains(OFFICE_PRODUCT)), "one PI product for the four apps");

    hook.sign_out();
    wait_lock(&hook, "sign-out", Some(EditLock::SignIn));
}

#[test]
fn the_personal_context_asks_for_an_organization() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "the lease", None);
    hook.select_org(None);
    wait_for(&hook, "personal context", |s| matches!(&s.view, AccountView::SignedIn(v) if v.active_org.is_none()));
    assert_eq!(hook.edit_lock(), Some(EditLock::ChooseOrg));
    hook.select_org(Some(ORG_OFFICE.into()));
    wait_lock(&hook, "back in the free organization", None);
}

#[test]
fn a_revoked_device_is_locked_after_the_next_sync() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "the lease", None);
    let device = account(&hook).diagnostics().device_id.unwrap();
    assert!(env.server().revoke_device(&device));
    env.advance(13 * 3600);
    let _ = account(&hook).refresh();
    assert_eq!(licence(&hook), Some(LicenceState::DeviceRevoked));
    assert_eq!(hook.edit_lock(), Some(EditLock::DeviceRevoked));
    assert!(!EditLock::DeviceRevoked.retry_offered());
}

#[test]
fn an_expired_lease_offline_asks_to_reconnect() {
    let mut env = Env::new(Scenario::office_free());
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "the lease", None);
    // PI is unreachable, and the 7-day lease runs out.
    drop(env.server.take());
    env.advance(8 * DAY);
    assert!(account(&hook).refresh().is_err());
    assert!(matches!(licence(&hook), Some(LicenceState::Inactive { .. })), "{:?}", licence(&hook));
    assert_eq!(hook.edit_lock(), Some(EditLock::RenewOffline));
}

#[test]
fn retry_activates_once_the_organization_has_a_plan() {
    let env = Env::new(Scenario { tenants: Vec::new(), ..Scenario::office_free() });
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "no plan", Some(EditLock::NoPlan));
    assert_eq!(licence(&hook), Some(LicenceState::NotEntitled));
    assert!(EditLock::NoPlan.retry_offered());
    // PI gave the organization its free plan; a plain sync does not see it (ADR 0013), Retry does.
    env.server().update_scenario(|s| s.tenants = vec![Tenant::office_free("ten_office", ORG_OFFICE)]);
    let _ = account(&hook).refresh();
    assert_eq!(hook.edit_lock(), Some(EditLock::NoPlan));
    hook.retry_activation();
    wait_lock(&hook, "the retried activation", None);
}

#[test]
fn retry_lifts_the_device_cap_once_a_device_is_removed() {
    let env = Env::new(Scenario { tenants: vec![Tenant::office_free("ten_office", ORG_OFFICE).device_cap(0)], ..Scenario::office_free() });
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "the device cap", Some(EditLock::DeviceCap));
    env.server().update_scenario(|s| s.tenants[0].device_cap = 2);
    let _ = account(&hook).refresh();
    assert_eq!(hook.edit_lock(), Some(EditLock::DeviceCap), "a plain sync keeps the block (lease.md §6)");
    hook.retry_activation();
    wait_lock(&hook, "the retried activation", None);
}

#[test]
fn a_refused_edit_syncs_without_waiting_for_the_hourly_refresh() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/api/v1/id/devices/activate", 422, "")], ..Scenario::office_free() });
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "a failed activation", Some(EditLock::NotActivated));
    // PI recovered; nothing asks for a sync except the refused edit itself.
    env.server().clear_faults();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(hook.edit_lock(), Some(EditLock::NotActivated));
    hook.edit_refused();
    wait_lock(&hook, "the resync to activate this device", None);
}

#[test]
fn the_lock_answers_from_cached_state_while_pi_hangs() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    sign_in(&hook);
    wait_lock(&hook, "the lease", None);
    for (method, path) in [
        ("POST", "/id/token"),
        ("GET", "/api/v1/id/me"),
        ("GET", "/api/v1/id/me/entitlements"),
        ("GET", "/api/v1/id/devices/challenge"),
        ("POST", "/api/v1/id/devices/activate"),
        ("POST", "/api/v1/id/licenses/refresh"),
    ] {
        env.server().add_fault(Fault::delay(method, path, Duration::from_secs(5)));
    }
    // The worker is now stuck in a sync with PI.
    hook.retry_activation();
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    assert_eq!(hook.edit_lock(), None);
    assert!(started.elapsed() < Duration::from_millis(500), "took {:?}", started.elapsed());
}

/// A signed-in status with `licence` in organization `org`.
fn status(org: Option<&str>, licence: LicenceState, online: bool) -> Status {
    Status::SignedIn(Box::new(SignedIn {
        user: None,
        orgs: Vec::new(),
        active_org: org.map(str::to_owned),
        licence,
        licences: Default::default(),
        warning: None,
        online,
        session_only: false,
        reauth_required: false,
    }))
}

#[test]
fn the_denial_order_follows_ubotcad() {
    use licence::{lock_in, sync_may_lift};
    let active =
        |features: &[&str]| LicenceState::Active { lease_id: "l".into(), features: features.iter().map(|f| f.to_string()).collect(), expires_at: T0 };
    let org = Some(ORG_OFFICE);
    assert_eq!(lock_in(&Status::Guest, |_| true), Some(EditLock::SignIn));
    assert_eq!(lock_in(&Status::SigningIn, |_| true), Some(EditLock::SignIn));
    assert_eq!(lock_in(&status(None, active(&[OFFICE_USE]), true), |_| true), Some(EditLock::ChooseOrg), "personal context first");
    for (state, lock) in [
        (LicenceState::DeviceRevoked, EditLock::DeviceRevoked),
        (LicenceState::DeviceCapReached, EditLock::DeviceCap),
        (LicenceState::NotEntitled, EditLock::NoPlan),
        (LicenceState::NotActivated, EditLock::NotActivated),
        (LicenceState::SessionOnly, EditLock::SessionOnly),
    ] {
        assert_eq!(lock_in(&status(org, state.clone(), true), |_| true), Some(lock), "{state:?}");
    }
    let expired = LicenceState::Inactive { lease_id: "l".into(), expires_at: T0 };
    assert_eq!(lock_in(&status(org, expired.clone(), false), |_| true), Some(EditLock::RenewOffline));
    assert_eq!(lock_in(&status(org, expired, true), |_| true), Some(EditLock::RenewFailed));
    assert_eq!(lock_in(&status(org, active(&[OFFICE_USE]), true), |_| true), None);
    assert_eq!(lock_in(&status(org, active(&[OFFICE_USE]), true), |_| false), Some(EditLock::NoPlan), "allowed() must agree");
    let feature_ids: Vec<String> = FREE_OFFICE.iter().map(|code| format!("{OFFICE_PRODUCT}.{code}")).collect();
    assert!(feature_ids.iter().any(|f| f == OFFICE_USE), "pi-mock's free plan grants {OFFICE_USE}");
    assert_eq!(lock_in(&status(org, active(&["erpconsoffice.view", "erpconsoffice.export"]), true), |_| true), Some(EditLock::NoPlan));
    assert!(sync_may_lift(EditLock::RenewFailed) && sync_may_lift(EditLock::RenewOffline) && sync_may_lift(EditLock::NotActivated));
    assert!(!sync_may_lift(EditLock::DeviceCap) && EditLock::DeviceCap.retry_offered());
    for lock in [EditLock::SignIn, EditLock::DeviceRevoked, EditLock::SessionOnly, EditLock::Unavailable] {
        assert!(!sync_may_lift(lock) && !lock.retry_offered(), "{lock:?}");
    }
}

#[test]
fn an_account_that_cannot_open_locks_editing() {
    let env = Env::new(Scenario::office_free());
    let mut config = mock_config(env.server(), &env.dir);
    // `Account::new` refuses a configuration that is not the Office profile's.
    config.client_id = "another-client".into();
    let hook = DesktopAccount::new(config, Arc::new(MemorySecretStore::new()), Box::new(follow_redirect));
    assert_eq!(hook.edit_lock(), Some(EditLock::SignIn), "before the worker opened it, as a guest");
    hook.attach(&egui::Context::default());
    let until = Instant::now() + Duration::from_secs(10);
    while hook.edit_lock() != Some(EditLock::Unavailable) {
        assert!(Instant::now() < until, "still {:?}", hook.edit_lock());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_build_configuration_turns_licensing_on() {
    let config = config::config_for("http://127.0.0.1:9");
    assert!(config.offline_licensing);
    assert_eq!(config.product, erpcons_account::Product::Pdf);
    assert_eq!(config.client_id, "erpcons-office-desktop");
    let pinned: Vec<&str> = erpcons_account::PLATFORM_KEYS.iter().map(|(kid, _)| *kid).collect();
    assert!(pinned.contains(&"lease-2026-10"), "the production lease key is pinned: {pinned:?}");
    assert_eq!(config.platform_keys.iter().map(|k| k.kid.as_str()).collect::<Vec<_>>(), pinned, "and no other key");
}

#[cfg(debug_assertions)]
#[test]
fn the_dev_lease_key_parses_only_kid_and_64_hex_digits() {
    use config::parse_dev_lease_key;
    let hex: String = pi_mock::dev_lease_public_key().iter().map(|b| format!("{b:02x}")).collect();
    let key = parse_dev_lease_key(&format!("{}:{hex}", pi_mock::DEV_LEASE_KID));
    assert_eq!(key, Some(PinnedKey { kid: pi_mock::DEV_LEASE_KID.into(), key: pi_mock::dev_lease_public_key() }));
    for bad in ["", "kid", ":00", "kid:00", &format!("kid:{hex}0"), &format!("kid:{}", "zz".repeat(32)), &format!(":{hex}")] {
        assert_eq!(parse_dev_lease_key(bad), None, "{bad:?}");
    }
}
