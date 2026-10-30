//! Where the desktop app finds PI ID and which keys verify its leases.

use erpcons_account::{Config, Product};

/// This build's account configuration; `None` without `ERPCONS_PI_ISSUER` (no account, no gate).
///
/// Release builds verify leases with erpcons-account's pinned `PLATFORM_KEYS` only. Debug builds
/// also pin `ERPCONS_PI_DEV_LEASE_KEY` (`kid:<64 hex digits>`, read at run time) to run against
/// pi-mock.
pub(super) fn pi_config() -> Option<Config> {
    let issuer = option_env!("ERPCONS_PI_ISSUER").filter(|issuer| !issuer.trim().is_empty())?;
    let mut config = config_for(issuer);
    config.api_base = option_env!("ERPCONS_PI_API_BASE").filter(|base| !base.trim().is_empty()).map(Into::into);
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var("ERPCONS_PI_DEV_LEASE_KEY").ok().filter(|v| !v.trim().is_empty()) {
        match parse_dev_lease_key(value.trim()) {
            Some(key) => config.platform_keys.push(key),
            None => log::warn!("ERPCONS_PI_DEV_LEASE_KEY is not kid:<64 hex digits>; ignored"),
        }
    }
    Some(config)
}

pub(super) fn config_for(issuer: &str) -> Config {
    let mut config = Config::new(issuer, Product::Pdf, env!("CARGO_PKG_VERSION"));
    // Editing needs this device's lease of PI product `erpconsoffice` (erpcons-account ADR 0013).
    config.offline_licensing = true;
    config
}

/// `kid:hex32`: a lease-signing key id and its Ed25519 public key in 64 hex digits.
#[cfg(debug_assertions)]
pub(super) fn parse_dev_lease_key(value: &str) -> Option<erpcons_account::PinnedKey> {
    let (kid, hex) = value.split_once(':')?;
    // `from_str_radix` alone would accept a leading `+`.
    if kid.is_empty() || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(erpcons_account::PinnedKey { kid: kid.to_owned(), key })
}
