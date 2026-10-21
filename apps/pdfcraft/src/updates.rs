//! Asks GitHub for the latest PdfCraft release (Help ▸ Check for updates, issue #28).

use std::time::Duration;

use pdfcraft_ui_egui::i18n;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release};

const LATEST: &str = "https://api.github.com/repos/tuananhfr/pdfcraft/releases/latest";

/// The latest release. The answer is untrusted: its size is capped, and only a page under
/// [`RELEASES_PAGE`] is ever offered for download (anything else falls back to that list).
pub fn latest_release() -> Result<Release, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        // Read error statuses ourselves: a 403 from GitHub usually means its anonymous
        // rate limit ran out, and the retry time rides along in a response header.
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| i18n::fmt(i18n::t("Couldn't reach GitHub ({e})"), &[("e", &e.to_string())]))?;
    let status = response.status().as_u16();
    if status == 403 {
        let reset = response.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok());
        return Err(rate_limited(reset, now_unix()));
    }
    if !(200..300).contains(&status) {
        return Err(i18n::fmt(i18n::t("Couldn't reach GitHub (HTTP status: {status})"), &[("status", &status.to_string())]));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .map_err(|e| i18n::fmt(i18n::t("Unreadable answer ({e})"), &[("e", &e.to_string())]))?;
    parse(&body)
}

/// The certificate authorities the operating system trusts.
fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err(i18n::t("No trusted certificates found on this system").into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// The current Unix time in seconds (0 when the clock gives nothing usable).
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What to tell the user when GitHub answers 403: its anonymous quota (60 checks an hour,
/// shared by everyone behind the same address) ran out. `reset` is the `x-ratelimit-reset`
/// header, the Unix time the quota refills; `now` is the current Unix time. Anything
/// surprising (no header, not a number, already past) falls back to asking for patience.
fn rate_limited(reset: Option<&str>, now: u64) -> String {
    let fallback = i18n::t("GitHub is limiting update checks for now (shared hourly quota); try again later");
    let wait = reset.and_then(|r| r.trim().parse::<u64>().ok()).and_then(|reset| reset.checked_sub(now)).filter(|&wait| wait > 0);
    match wait {
        Some(wait) => {
            let minutes = wait.div_ceil(60).max(1);
            i18n::trn(
                i18n::current(),
                minutes,
                "GitHub is limiting update checks for now (shared hourly quota); try again in about {n} minute",
                "GitHub is limiting update checks for now (shared hourly quota); try again in about {n} minutes",
            )
        }
        None => fallback.into(),
    }
}

fn parse(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| i18n::fmt(i18n::t("Unreadable answer ({e})"), &[("e", &e.to_string())]))?;
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).ok_or(i18n::t("No release found"))?.to_string();
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.strip_prefix(RELEASES_PAGE).is_some_and(|rest| rest.starts_with('/') && !rest.contains(['?', '#', '\\'])))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    Ok(Release { version, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_and_only_our_release_pages_are_offered() {
        let r = parse(r#"{"tag_name":"v0.2.0","html_url":"https://github.com/tuananhfr/pdfcraft/releases/tag/v0.2.0"}"#).unwrap();
        assert_eq!(r, Release { version: "v0.2.0".into(), url: "https://github.com/tuananhfr/pdfcraft/releases/tag/v0.2.0".into() });
        for elsewhere in ["https://example.com/pdfcraft.exe", "https://github.com/tuananhfr/pdfcraft/releases.evil/x", "javascript:alert(1)"] {
            let r = parse(&format!(r#"{{"tag_name":"v9.9.9","html_url":"{elsewhere}"}}"#)).unwrap();
            assert_eq!(r.url, RELEASES_PAGE, "{elsewhere}");
        }
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn rate_limit_messages_name_the_wait() {
        let limited = "GitHub is limiting update checks for now (shared hourly quota); try again in about 20 minutes";
        assert_eq!(rate_limited(Some("1800001200"), 1800000000), limited);
        // Rounds up: 61 seconds out reads as 2 minutes.
        assert_eq!(
            rate_limited(Some("1800000061"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 2 minutes"
        );
        assert_eq!(
            rate_limited(Some("1800000060"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 1 minute"
        );
        // No header, not a number, or already past: ask for patience instead of a time.
        let later = "GitHub is limiting update checks for now (shared hourly quota); try again later";
        for bad in [None, Some(""), Some("soon"), Some("1799999999"), Some("1800000000")] {
            assert_eq!(rate_limited(bad, 1800000000), later);
        }
    }

    #[test]
    fn vietnamese_update_errors_and_rate_limits() {
        let previous = i18n::current();
        i18n::set_current(i18n::Lang::from_pref("vi"));
        assert_eq!(parse("{}").unwrap_err(), "Không tìm thấy bản phát hành");
        assert!(parse("<html>").unwrap_err().starts_with("Không đọc được phản hồi ("));
        assert_eq!(
            rate_limited(Some("61"), 0),
            "GitHub đang giới hạn lượt kiểm tra cập nhật (hạn mức dùng chung mỗi giờ); hãy thử lại sau khoảng 2 phút"
        );
        assert_eq!(rate_limited(None, 0), "GitHub đang giới hạn lượt kiểm tra cập nhật (hạn mức dùng chung mỗi giờ); hãy thử lại sau");
        i18n::set_current(previous);
    }

    /// Live: asks GitHub over TLS with the OS's roots (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn github_answers_with_the_latest_release() {
        let r = latest_release().unwrap();
        assert!(pdfcraft_ui_egui::updates::is_newer(&r.version, "0.0.0"), "{r:?}");
        assert!(r.url.starts_with(RELEASES_PAGE), "{r:?}");
    }
}
