//! What logs may say about the user's files (spec §07): a document's name, path or URL never,
//! only its extension and size; the app's own folders (settings, fonts, logs) with the home
//! folder written as `~`, which keeps the account name out.

use std::path::Path;

/// A document as a log line may name it: ".pdf (12345 bytes)", ".pdf" without a size, and
/// "file" when the name has no plausible extension. `name` may be a name, a path or a URL.
pub fn document(name: &str, bytes: Option<u64>) -> String {
    let kind = extension(name).map_or_else(|| "file".to_string(), |e| format!(".{e}"));
    match bytes {
        Some(n) => format!("{kind} ({n} bytes)"),
        None => kind,
    }
}

/// [`document`] for a file on disk, sized from its metadata when it can be read.
pub fn document_path(path: &Path) -> String {
    document(&path.to_string_lossy(), std::fs::metadata(path).ok().map(|m| m.len()))
}

/// The extension of the last path segment of `name`, lower-cased; a URL's query and fragment
/// don't count. Long or non-alphanumeric "extensions" are someone's words, not a file type.
fn extension(name: &str) -> Option<String> {
    let path = name.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit(['/', '\\']).next().unwrap_or_default();
    let (stem, ext) = last.rsplit_once('.')?;
    (!stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())).then(|| ext.to_ascii_lowercase())
}

/// One of the app's own folders or files (settings, fonts, logs, the control file) with the
/// user's home folder written as `~`.
pub fn home_relative(path: &Path) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from);
    relative_to(path, home.as_deref().filter(|h| !h.as_os_str().is_empty()))
}

fn relative_to(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_named_by_extension_and_size_only() {
        assert_eq!(document("Báo cáo quý 3.PDF", Some(12_345)), ".pdf (12345 bytes)");
        assert_eq!(document("/home/an/Hợp đồng/khach.pdf", None), ".pdf");
        assert_eq!(document(r"C:\Users\an\Desktop\offer.pdf", Some(1)), ".pdf (1 bytes)");
        assert_eq!(document("https://example.com/files/secret.pdf?token=abc#page=2", None), ".pdf");
        assert_eq!(document("https://example.com/download?name=x.pdf", None), "file", "the query is not the name");
        assert_eq!(document("README", Some(3)), "file (3 bytes)");
        assert_eq!(document(".pdf", None), "file");
        assert_eq!(document("notes.my secret words", None), "file");
        assert_eq!(document("", None), "file");
    }

    #[test]
    fn the_home_folder_is_written_as_a_tilde() {
        let home = Path::new("/home/an");
        let settings = relative_to(Path::new("/home/an/.config/pdf/language"), Some(home));
        assert!(settings.starts_with('~') && settings.ends_with("language") && !settings.contains("home"), "{settings}");
        assert_eq!(relative_to(home, Some(home)), "~");
        assert_eq!(relative_to(Path::new("/home/anna/x"), Some(home)), "/home/anna/x", "a sibling folder is not inside home");
        assert_eq!(relative_to(Path::new("/usr/share/fonts/a.ttf"), Some(home)), "/usr/share/fonts/a.ttf");
        assert_eq!(relative_to(Path::new("/tmp/x"), None), "/tmp/x");
    }
}
