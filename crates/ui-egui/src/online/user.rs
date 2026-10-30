//! `setUser` of `erpHost` v2: the signed-in account names the author of what the user creates
//! (comments, markups, replies, redaction marks) in place of the local preference, which is kept
//! and saved as it was.

use std::sync::{Arc, Mutex};

use crate::{MAX_AUTHOR_CHARS, PdfCraftApp};

/// The latest `setUser` not applied yet: the account's name, or `None` for `setUser(null)`.
pub type UserInbox = Arc<Mutex<Option<Option<String>>>>;

impl PdfCraftApp {
    /// Apply the latest `setUser`. `null`, or an account without a name, brings back the local
    /// preference.
    pub(crate) fn apply_host_user(&mut self) {
        let Some(online) = self.online.as_mut() else { return };
        let Some(change) = online.user.lock().ok().and_then(|mut u| u.take()) else { return };
        let name = change.map(|n| n.trim().chars().take(MAX_AUTHOR_CHARS).collect::<String>()).filter(|n| !n.is_empty());
        match name {
            Some(name) => {
                online.local_author.get_or_insert_with(|| self.comment_prefs.author.clone());
                self.comment_prefs.author = name;
            }
            None => {
                if let Some(local) = online.local_author.take() {
                    self.comment_prefs.author = local;
                }
            }
        }
    }

    /// The author the settings keep: the local preference, also while an account's name stands in.
    pub(crate) fn saved_author(&self) -> &str {
        self.online.as_ref().and_then(|o| o.local_author.as_deref()).unwrap_or(&self.comment_prefs.author)
    }
}
