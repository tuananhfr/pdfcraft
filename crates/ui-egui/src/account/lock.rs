//! The licence lock: while the host's [`super::AccountHook::edit_lock`] says editing is refused (the
//! desktop app without a PI lease), every open document is in the session's view-only mode, the
//! same one the office shell's view mode uses, with the licence reason as its notice. Opening,
//! viewing, saving, exporting and printing stay; lifting the lock needs no reopening.

use pdfcraft_engine::{DocId, SessionLimits};

use crate::PdfCraftApp;

/// Why the host refuses edits; each says what the user can do about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditLock {
    /// Signed out, or a sign-in is waiting for the browser.
    SignIn,
    /// The personal context: PI licenses organizations only.
    ChooseOrg,
    DeviceRevoked,
    DeviceCap,
    NotActivated,
    /// No OS credential store: the device key can't be kept, so there is no lease either.
    SessionOnly,
    /// The lease ran out while the app is offline.
    RenewOffline,
    /// The lease ran out while the app is online: the renewal failed or has not run yet.
    RenewFailed,
    /// The organization's plan does not grant editing.
    NoPlan,
    /// The account could not be opened on this computer.
    Unavailable,
}

impl EditLock {
    /// The notice of a locked document and the Account dialog's line, in the interface language.
    pub fn reason(self) -> &'static str {
        match self {
            EditLock::SignIn => tl!("Sign in to PI ID to edit."),
            EditLock::ChooseOrg => tl!("Choose an organization to edit."),
            EditLock::DeviceRevoked => tl!("This device was removed from your account. Sign in again to edit."),
            EditLock::DeviceCap => tl!("Your account has reached its device limit. Remove a device in PI ID, then press Retry in Account."),
            EditLock::NotActivated | EditLock::SessionOnly => tl!("This device is not activated for editing yet."),
            EditLock::RenewOffline => tl!("Connect to the internet to renew your license and edit."),
            EditLock::RenewFailed => tl!("Your license could not be renewed yet. The app keeps trying."),
            EditLock::NoPlan => tl!("This organization has no Office plan for editing."),
            EditLock::Unavailable => tl!("PI ID licensing is unavailable on this computer, so editing is off."),
        }
    }

    /// The Account dialog offers Retry: a sync may lift it, and a device-cap block lifts only on
    /// the user's retry (erpcons-account `lease.md` §6).
    pub fn retry_offered(self) -> bool {
        matches!(
            self,
            EditLock::ChooseOrg | EditLock::DeviceCap | EditLock::NotActivated | EditLock::RenewOffline | EditLock::RenewFailed | EditLock::NoPlan
        )
    }
}

impl super::AccountState {
    pub(crate) fn press_lock_button(&mut self, sign_in: bool) {
        self.lock_button = Some(sign_in);
    }
}

impl PdfCraftApp {
    /// Read the hook's lock and put it on, or take it off, every open document. Runs every frame;
    /// without a lock now or before (the web, a licensed desktop) it touches nothing.
    pub(crate) fn apply_licence_lock(&mut self) {
        let Some(hook) = self.account_hook.clone() else { return };
        if let Some(sign_in) = self.account.lock_button.take() {
            self.open_account_for_lock(sign_in);
        }
        let lock = hook.edit_lock();
        self.account.lock = lock;
        if lock.is_none() && !self.account.lock_applied {
            return;
        }
        // An edit already started in a dialog finishes first; the document underneath takes no
        // other input meanwhile.
        if lock.is_some() && self.modal_open() && self.has_editable_document() {
            return;
        }
        self.account.lock_applied = lock.is_some();
        let ids: Vec<DocId> = self.views.iter().map(|v| v.id).collect();
        for id in ids {
            let Some(limits) = self.session.get(id).map(pdfcraft_engine::Document::limits) else { continue };
            let view_only = lock.is_some() || self.host_read_only(id);
            if limits.view_only == view_only {
                continue;
            }
            if view_only {
                // Text still being typed is kept, as when the office shell downgrades a document.
                self.commit_inline_edits(id);
            }
            self.session.set_limits(id, SessionLimits { view_only, ..limits });
        }
    }

    fn has_editable_document(&self) -> bool {
        self.views.iter().any(|v| self.session.get(v.id).is_some_and(|d| !d.limits().view_only))
    }

    /// The licence lock that makes document `id` view-only, if that is what does.
    pub(crate) fn licence_locked(&self, id: DocId) -> Option<EditLock> {
        self.account.lock.filter(|_| self.account.lock_applied && !self.host_read_only(id))
    }

    /// A locked document's notice button: the Account dialog, with a sign-in started for
    /// [`EditLock::SignIn`] unless one is already waiting for the browser.
    fn open_account_for_lock(&mut self, sign_in: bool) {
        if sign_in
            && self.account.snapshot.view != super::AccountView::SigningIn
            && let Some(hook) = &self.account_hook
        {
            hook.sign_in();
        }
        self.show_account();
    }

    /// The licence lock refused an edit: the host may sync, since a new lease would lift it.
    pub(crate) fn licence_refused(&self) {
        if self.account.lock_applied
            && let Some(hook) = &self.account_hook
        {
            hook.edit_refused();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EditLock;
    use crate::i18n::{self, Lang};

    #[test]
    fn reasons_read_short_in_vietnamese() {
        i18n::set_current(Lang::from_pref("vi"));
        for (lock, vi) in [
            (EditLock::SignIn, "Đăng nhập PI ID để sửa."),
            (EditLock::ChooseOrg, "Chọn tổ chức để sửa."),
            (EditLock::DeviceRevoked, "Thiết bị này đã bị gỡ khỏi tài khoản. Đăng nhập lại để sửa."),
            (EditLock::DeviceCap, "Tài khoản đã đạt giới hạn số thiết bị. Hãy gỡ bớt một thiết bị trong PI ID rồi bấm Thử lại trong Tài khoản."),
            (EditLock::NotActivated, "Thiết bị này chưa được kích hoạt để sửa."),
            (EditLock::SessionOnly, "Thiết bị này chưa được kích hoạt để sửa."),
            (EditLock::RenewOffline, "Kết nối Internet để gia hạn giấy phép và sửa tiếp."),
            (EditLock::RenewFailed, "Chưa gia hạn được giấy phép. Ứng dụng sẽ tự thử lại."),
            (EditLock::NoPlan, "Tổ chức này chưa có gói Office để sửa."),
            (EditLock::Unavailable, "Không dùng được giấy phép PI ID trên máy này nên không sửa được."),
        ] {
            assert_eq!(lock.reason(), vi, "{lock:?}");
        }
        i18n::set_current(Lang::EN);
        assert_eq!(EditLock::SignIn.reason(), "Sign in to PI ID to edit.");
    }
}
