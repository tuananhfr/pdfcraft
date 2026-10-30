//! Optional PI ID account (ERPCons): the Account dialog, the tab strip's account button, Help ▸
//! Account and the paid licence expiry notice. The host installs an [`AccountHook`] (desktop:
//! `erpcons-account` on a worker thread; web: the office shell's `window.erpAccount`). Without one
//! nothing here shows and every feature is allowed.
//!
//! Only account state crosses the hook: never a document, its name or its path.

mod lock;
mod page;

pub use lock::EditLock;
pub(crate) use page::{body, help_menu_item, strip_button, warning_notice};

use crate::{Dialog, PdfCraftApp};

/// Implemented by the native and web adapters. Every method returns at once; work runs on the adapter's own
/// worker (thread on desktop, promise on web) and the result shows up in the next `snapshot()`.
pub trait AccountHook {
    fn snapshot(&self) -> AccountSnapshot;
    fn sign_in(&self);
    fn cancel_sign_in(&self);
    fn sign_out(&self);
    fn select_org(&self, org_id: Option<String>);
    fn retry_activation(&self);
    fn allowed(&self, feature: &str) -> bool;
    fn warning_shown(&self, days: u32);
    /// Called once with the egui context so the adapter can request a repaint when its snapshot changes.
    fn attach(&self, ctx: &egui::Context);
    /// Whether this host gates editing on a licence at all (the desktop app with PI licensing).
    fn gates_editing(&self) -> bool {
        false
    }
    /// Why editing is refused now, from cached state only: it is asked every frame. `None` (the
    /// web) never locks.
    fn edit_lock(&self) -> Option<EditLock> {
        None
    }
    /// An edit was just refused for [`Self::edit_lock`]; the adapter may sync to lift it.
    fn edit_refused(&self) {}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub view: AccountView,
    /// A command the user started is still running.
    pub busy: bool,
    pub error: Option<AccountError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AccountView {
    #[default]
    Guest,
    SigningIn,
    SignedIn(SignedInView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInView {
    pub name: String,
    pub email: Option<String>,
    pub orgs: Vec<OrgView>,
    /// `None` is the personal context.
    pub active_org: Option<String>,
    pub licence: LicenceView,
    pub warning: Option<WarningView>,
    pub online: bool,
    pub session_only: bool,
    pub reauth_required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgView {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenceView {
    /// No non-free entitlement (normal today).
    Free,
    /// Desktop lease.
    Active {
        expires_at: i64,
        pro: bool,
    },
    /// Web: no lease, entitlements read online.
    Online {
        pro: bool,
    },
    Inactive {
        expires_at: i64,
    },
    DeviceRevoked,
    DeviceCapReached,
    SessionOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarningView {
    pub days: u32,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    Cancelled,
    PopupBlocked,
    Network,
    Timeout,
    Denied,
    RateLimited { retry_after_secs: Option<u64> },
    DeviceCap,
    DeviceRevoked,
    Suspended,
    OrgUnavailable,
    ReauthRequired,
    ClientTooOld,
    ServiceUnavailable,
    Other,
}

/// Whether `feature` needs an entitlement or a lease. Only `.view` and `.export` are pinned free;
/// `erpconsoffice.use` (editing) is `pro` from erpcons-account 0.5.0 (ADR 0013).
pub fn is_pro_feature(feature: &str) -> bool {
    !(feature.ends_with(".view") || feature.ends_with(".export"))
}

/// Whether holding `feature` means a paid plan: `pro`, but not `.use`, which the free Office plan
/// grants too.
pub fn is_paid_feature(feature: &str) -> bool {
    is_pro_feature(feature) && !feature.ends_with(".use")
}

/// The avatar letter: the given name's first letter (Vietnamese names put it last, as the office
/// shell does), else the email's.
pub fn avatar_letter(name: &str, email: Option<&str>) -> Option<String> {
    let given = name.split_whitespace().next_back().unwrap_or_default();
    given.chars().chain(email.unwrap_or_default().chars()).find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect())
}

/// What the app keeps between frames about the account.
#[derive(Default)]
pub(crate) struct AccountState {
    attached: bool,
    /// The hook's snapshot, read once per frame.
    snapshot: AccountSnapshot,
    /// The expiry warning closed in this run; hidden even before the adapter's snapshot drops it.
    dismissed: Option<WarningView>,
    /// The hook's [`AccountHook::edit_lock`], read once per frame.
    lock: Option<EditLock>,
    /// The open documents are in view-only mode for `lock` (put on later than `lock` while a
    /// dialog is open).
    lock_applied: bool,
    /// A locked document's notice button was pressed; whether it signs in. Handled next frame,
    /// since the page that draws it holds the document.
    lock_button: Option<bool>,
}

impl PdfCraftApp {
    /// Whether `feature` may be used. True without an account hook; editing is gated through
    /// [`AccountHook::edit_lock`] instead.
    pub fn allowed(&self, feature: &str) -> bool {
        self.account_hook.as_ref().is_none_or(|hook| hook.allowed(feature))
    }

    /// Opens the Account dialog; does nothing without an account hook.
    pub fn show_account(&mut self) {
        if self.account_hook.is_some() {
            self.dialog = Some(Dialog::Account);
        }
    }

    pub(crate) fn poll_account(&mut self, ctx: &egui::Context) {
        let Some(hook) = &self.account_hook else { return };
        if !self.account.attached {
            self.account.attached = true;
            hook.attach(ctx);
        }
        self.account.snapshot = hook.snapshot();
    }
}

/// A local calendar date: `dd/mm/yyyy` in Vietnamese, Preferences ▸ Date format otherwise.
pub fn date_text(session: &pdfcraft_engine::Session, unix_secs: i64) -> String {
    use pdfcraft_engine::dates;
    let Some((y, m, d)) = dates::local_day(unix_secs) else { return String::new() };
    let interface = crate::i18n::current().code();
    if interface == "vi" {
        return format!("{d:02}/{m:02}/{y:04}");
    }
    let lang = dates::date_language(session.date_language().unwrap_or(interface)).unwrap_or(&dates::DATE_LANGUAGES[0]);
    dates::format_day(session.date_format(), (y, m, d), lang)
}

/// The error line in the current language; `None` for a cancelled sign-in, which needs no message.
pub fn error_text(error: AccountError) -> Option<String> {
    let text = match error {
        AccountError::Cancelled => return None,
        AccountError::PopupBlocked => tl!("The browser blocked the sign-in window. Allow pop-ups for this site and try again."),
        AccountError::Network => tl!("Could not reach PI ID. Check your connection and try again."),
        AccountError::Timeout => tl!("PI ID took too long to answer. Try again."),
        AccountError::Denied => tl!("PI ID refused the request. Free features keep working."),
        AccountError::RateLimited { retry_after_secs: Some(secs) } if secs > 0 => {
            let minutes = secs.div_ceil(60).to_string();
            return Some(crate::i18n::fmt(tl!("PI ID is busy. Try again in {minutes} min."), &[("minutes", &minutes)]));
        }
        AccountError::RateLimited { .. } => tl!("PI ID is busy. Try again in a few minutes."),
        AccountError::DeviceCap => tl!("Your account has reached its device limit. Remove a device in PI ID, then retry."),
        AccountError::DeviceRevoked => tl!("This device was removed from your account. Free features keep working."),
        AccountError::Suspended => tl!("Your PI ID account is suspended. Free features keep working."),
        AccountError::OrgUnavailable => tl!("This organization is not available to your account."),
        AccountError::ReauthRequired => tl!("Your session has expired. Sign in again."),
        AccountError::ClientTooOld => tl!("This version of the app is too old for PI ID. Update the app, then try again."),
        AccountError::ServiceUnavailable => tl!("PI ID is temporarily unavailable. Try again later."),
        AccountError::Other => tl!("Something went wrong with PI ID. Try again."),
    };
    Some(text.to_string())
}

/// The page the desktop sign-in shows in the browser tab once it returns, in the interface language.
pub fn callback_page(signed_in: bool) -> String {
    let text = if signed_in {
        tl!("Signed in to PI ID. You can close this tab and go back to the app.")
    } else {
        tl!("Sign-in did not finish. Close this tab and try again in the app.")
    };
    let text = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!doctype html><meta charset=\"utf-8\"><title>PI ID</title><p style=\"font:16px sans-serif;margin:3em\">{text}</p>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Lang};

    #[test]
    fn the_avatar_shows_the_given_name_like_the_office_shell() {
        assert_eq!(super::avatar_letter("Nguyễn Văn An", None).as_deref(), Some("A"));
        assert_eq!(super::avatar_letter(" ", Some("binh@example.com")).as_deref(), Some("B"));
        assert_eq!(super::avatar_letter("", None), None);
    }

    #[test]
    fn only_view_and_export_are_free_and_use_is_not_paid() {
        for free in ["erpconsoffice.view", "erpconsoffice.export", "vanban.view"] {
            assert!(!is_pro_feature(free) && !is_paid_feature(free), "{free}");
        }
        for pro in ["erpconsoffice.use", "pdf.use", "pdf.ocr", "pdf.redact", "pdf.use.extra", "pdf.viewer"] {
            assert!(is_pro_feature(pro), "{pro}");
        }
        assert!(!is_paid_feature("erpconsoffice.use"), "the free Office plan grants editing");
        for paid in ["pdf.ocr", "erpconsoffice.redact", "pdf.use.extra"] {
            assert!(is_paid_feature(paid), "{paid}");
        }
    }

    #[test]
    fn dates_follow_the_interface_language() {
        // 15 March 2027 at noon UTC: the same calendar day in the time zones the tests run in.
        let noon = 1_805_112_000;
        let session = pdfcraft_engine::Session::new();
        i18n::set_current(Lang::from_pref("vi"));
        assert_eq!(date_text(&session, noon), "15/03/2027");
        i18n::set_current(Lang::EN);
        assert_eq!(date_text(&session, noon), "3/15/2027", "the default Preferences ▸ Date format");
        assert_eq!(date_text(&session, i64::MAX), "");
    }

    #[test]
    fn a_cancelled_sign_in_says_nothing_and_rate_limits_say_when_to_retry() {
        i18n::set_current(Lang::EN);
        assert_eq!(error_text(AccountError::Cancelled), None);
        assert_eq!(error_text(AccountError::RateLimited { retry_after_secs: Some(90) }).as_deref(), Some("PI ID is busy. Try again in 2 min."));
        assert_eq!(
            error_text(AccountError::RateLimited { retry_after_secs: Some(0) }).as_deref(),
            Some("PI ID is busy. Try again in a few minutes.")
        );
        i18n::set_current(Lang::from_pref("vi"));
        assert_eq!(error_text(AccountError::RateLimited { retry_after_secs: Some(1) }).as_deref(), Some("PI ID đang bận. Hãy thử lại sau 1 phút."));
        i18n::set_current(Lang::EN);
    }

    #[test]
    fn the_browser_pages_are_translated_and_escaped() {
        i18n::set_current(Lang::from_pref("vi"));
        assert!(callback_page(true).contains("Đã đăng nhập PI ID."));
        assert!(callback_page(false).contains("Chưa đăng nhập xong."));
        i18n::set_current(Lang::EN);
        let page = callback_page(true);
        assert!(page.starts_with("<!doctype html>") && page.contains("Signed in to PI ID."));
    }
}
