//! Logs never name the user's documents (spec §07): no log line in the app, the shared UI or
//! the engine interpolates something called a name, title, document id, path, URL or file unless it goes through
//! `log_safe`, or is on the allowlist below with its reason. Command-line tools print the paths
//! their user typed to the terminal; that is output, not a log, so `pdfcraft-cli` is not scanned.

use std::path::{Path, PathBuf};

const MACROS: [&str; 9] = [
    "log::error!(",
    "log::warn!(",
    "log::info!(",
    "log::debug!(",
    "log::trace!(",
    "eprintln!(",
    "console::log_1(",
    "console::warn_1(",
    "console::error_1(",
];

/// (file suffix, text in the call, why it is not a document).
const ALLOWED: [(&str, &str, &str); 2] =
    [("apps/pdfcraft/src/main.rs", "adapter.name", "the GPU adapter's name"), ("apps/pdfcraft/src/main.rs", "info.name", "the GPU device's name")];

fn suspicious(word: &str) -> bool {
    // `title` and `doc_id`: how the office server's documents (erp-host) are known.
    const WORDS: [&str; 9] = ["name", "path", "url", "uri", "file", "filename", "dir", "title", "doc_id"];
    WORDS.contains(&word) || WORDS.iter().any(|w| word.ends_with(&format!("_{w}")))
}

/// The text of each macro call in `source` (outside `#[cfg(test)]` modules), up to its
/// closing parenthesis, with its line number.
fn calls(source: &str) -> Vec<(usize, String)> {
    let source = without_test_modules(source);
    let source = source.as_str();
    let mut out = Vec::new();
    for m in MACROS {
        let mut from = 0;
        while let Some(at) = source[from..].find(m).map(|i| i + from) {
            let body = &source[at + m.len()..];
            let (mut depth, mut in_str, mut escaped, mut end) = (1usize, false, false, body.len());
            for (i, c) in body.char_indices() {
                match (in_str, c) {
                    (true, '\\') if !escaped => escaped = true,
                    (true, '"') if !escaped => in_str = false,
                    (true, _) => escaped = false,
                    (false, '"') => in_str = true,
                    (false, '(') => depth += 1,
                    (false, ')') => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push((source[..at].matches('\n').count() + 1, body[..end].to_string()));
            from = at + m.len();
        }
    }
    out
}

/// What a call interpolates that sounds like a document: identifiers outside string literals
/// and `{placeholders}` inside them, plus `.display()`, in each argument not made by `log_safe`.
fn leaks(call: &str) -> Vec<String> {
    arguments(call).into_iter().filter(|a| !a.contains("log_safe::")).flat_map(argument_leaks).collect()
}

fn argument_leaks(arg: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut code = String::new();
    let mut rest = arg;
    while let Some(start) = rest.find('"') {
        code.push_str(&rest[..start]);
        let lit = &rest[start + 1..];
        let end = lit.char_indices().find(|&(i, c)| c == '"' && !lit[..i].ends_with('\\')).map_or(lit.len(), |(i, _)| i);
        for hole in lit[..end].split('{').skip(1).filter_map(|h| h.split_once('}')).map(|(h, _)| h) {
            words.push(hole.split(':').next().unwrap_or_default().to_string());
        }
        rest = lit.get(end + 1..).unwrap_or_default();
    }
    code.push_str(rest);
    words.extend(code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(str::to_string));
    let mut found: Vec<String> = words.into_iter().filter(|w| suspicious(w)).collect();
    if code.contains(".display()") {
        found.push(".display()".into());
    }
    found
}

/// `source` with each `#[cfg(test)] mod` (its braces, or up to `;`) blanked out, newlines kept
/// so line numbers stay right.
fn without_test_modules(source: &str) -> String {
    const MARK: &str = "#[cfg(test)]\nmod ";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find(MARK) {
        out.push_str(&rest[..at]);
        let module = &rest[at..];
        let end = module_end(module);
        out.extend(module[..end].chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        rest = &module[end..];
    }
    out.push_str(rest);
    out
}

/// The length of the module item at the start of `module`: through its matching `}`, or its `;`.
fn module_end(module: &str) -> usize {
    let bytes = module.as_bytes();
    let (mut depth, mut i) = (0usize, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b';' if depth == 0 => return i + 1,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            b'"' => i = string_end(bytes, i),
            b'r' if bytes.get(i + 1).is_some_and(|&b| b == b'#' || b == b'"')
                && !bytes.get(i.wrapping_sub(1)).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') =>
            {
                let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
                if bytes.get(i + 1 + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let from = i + 2 + hashes;
                    i = module[from..].find(&close).map_or(bytes.len(), |e| from + e + close.len() - 1);
                }
            }
            // A char literal such as '{' or '\'' (a lifetime has no closing quote two along).
            b'\'' if bytes.get(i + 1) == Some(&b'\\') => i = module[i + 3..].find('\'').map_or(i, |e| i + 3 + e),
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => i += 2,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// The index of the `"` closing the string literal opened at `open`.
fn string_end(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'"' => return i,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// The call's arguments, split at top-level commas.
fn arguments(call: &str) -> Vec<&str> {
    let bytes = call.as_bytes();
    let (mut out, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0usize, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' => i = string_end(bytes, i),
            b',' if depth == 0 => {
                out.push(&call[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&call[start..]);
    out
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if path.is_dir() {
            if !matches!(name, "tests" | "target" | "pdfcraft-cli") {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

#[test]
fn the_scan_tells_a_leak_from_a_safe_line() {
    assert_eq!(leaks(r#""{} is too large to keep for reopening", closed.name"#), ["name"]);
    assert_eq!(leaks(r#""reveal {}: {e}", path.display()"#), ["path", ".display()"]);
    assert_eq!(leaks(r#""could not fetch {url}: {e}""#), ["url"]);
    assert_eq!(leaks(r#""save of {} failed", record.doc_id"#), ["doc_id"]);
    assert_eq!(leaks(r#""opened {}", doc.title"#), ["title"]);
    assert_eq!(leaks(r#""{doc_id} is not open""#), ["doc_id"]);
    assert!(leaks(r#""a closed {} is too large", crate::log_safe::document(&closed.name, None)"#).is_empty());
    assert!(leaks(r#""no log file: {e}""#).is_empty(), "words of the message itself don't count");
    assert_eq!(
        leaks(r#""{} of {}", crate::log_safe::document(name, None), path.display()"#),
        ["path", ".display()"],
        "log_safe excuses only its own argument"
    );
    let found = calls(
        "fn f() {\n    log::warn!(\"x {}\", f(a, b));\n}\n#[cfg(test)]\nmod tests { fn g() { let _ = '{'; log::warn!(\"{path} }\"); } }\nfn h() { log::warn!(\"{url}\"); }\n#[cfg(test)]\nmod more;\nfn k() { log::info!(\"{dir}\"); }",
    );
    let mut found: Vec<(usize, &str)> = found.iter().map(|(line, call)| (*line, call.as_str())).collect();
    found.sort();
    assert_eq!(
        found,
        [(2, "\"x {}\", f(a, b)"), (6, "\"{url}\""), (9, "\"{dir}\"")],
        "calls in test modules are not logs of the app, and what follows one is"
    );
}

#[test]
fn logs_do_not_name_documents() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in [root.join("src"), root.join("../engine/src"), root.join("../../apps")] {
        sources(&dir, &mut files);
    }
    assert!(files.len() > 100, "the scan found only {} files", files.len());
    let mut bad = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
        let shown = file.to_string_lossy().replace('\\', "/");
        for (line, call) in calls(&source) {
            let allowed = |w: &String| ALLOWED.iter().any(|(f, text, _)| shown.ends_with(f) && call.contains(text) && text.ends_with(w.as_str()));
            let leaked: Vec<String> = leaks(&call).into_iter().filter(|w| !allowed(w)).collect();
            if !leaked.is_empty() {
                bad.push(format!("{shown}:{line}: {leaked:?} in {call}"));
            }
        }
    }
    assert!(bad.is_empty(), "log lines that may name a document (use log_safe, or allowlist with a reason):\n{}", bad.join("\n"));
}
