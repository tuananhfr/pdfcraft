//! Why editing is refused now: UbotCAD's denial order (cadcraft `licence/policy.rs`) for the Office
//! feature `erpconsoffice.use`, which a lease of this device must grant.

use erpcons_account::{LicenceState, OFFICE_USE, Status};
use pdfcraft_ui_egui::account::EditLock;

/// The lock of a status snapshot, `None` when editing is allowed. `allowed` is asked only while
/// the lease lists the feature, where it answers from the lease and requests no refresh.
pub(super) fn lock_in(status: &Status, allowed: impl FnOnce(&str) -> bool) -> Option<EditLock> {
    let signed_in = match status {
        Status::Guest | Status::SigningIn => return Some(EditLock::SignIn),
        Status::SignedIn(signed_in) => signed_in,
    };
    // PI licenses organizations only; an Office user gets a one-person one (D578).
    if signed_in.active_org.is_none() {
        return Some(EditLock::ChooseOrg);
    }
    // Online entitlements alone would grant too; only this device's lease shows a revoked or
    // capped device, so the lease must list the feature as well.
    match &signed_in.licence {
        LicenceState::DeviceRevoked => Some(EditLock::DeviceRevoked),
        LicenceState::DeviceCapReached => Some(EditLock::DeviceCap),
        LicenceState::NotEntitled => Some(EditLock::NoPlan),
        LicenceState::NotActivated => Some(EditLock::NotActivated),
        LicenceState::SessionOnly => Some(EditLock::SessionOnly),
        LicenceState::Inactive { .. } if !signed_in.online => Some(EditLock::RenewOffline),
        // Online yet expired: the renewal failed or has not run yet, which is not "no plan".
        LicenceState::Inactive { .. } => Some(EditLock::RenewFailed),
        LicenceState::Active { features, .. } if features.iter().any(|f| f == OFFICE_USE) && allowed(OFFICE_USE) => None,
        LicenceState::Active { .. } => Some(EditLock::NoPlan),
    }
}

/// A plain sync with PI may lift `lock`: activation retried, lease renewed, a plan bought or an
/// organization joined since the last sync. A device-cap block lifts only on the user's Retry.
pub(super) fn sync_may_lift(lock: EditLock) -> bool {
    lock.retry_offered() && lock != EditLock::DeviceCap
}
