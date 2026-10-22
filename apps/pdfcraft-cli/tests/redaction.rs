use std::process::Command;

#[test]
fn redaction_script_cannot_undo_after_export() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-cli-redaction-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("secret.txt"), "SECRET-123\nPublic line").unwrap();
    let steps = serde_json::json!([
        { "tool": "doc_create", "args": { "from": "text", "path": "secret.txt" } },
        { "tool": "doc_set_info", "args": { "doc": 1, "key": "Title", "value": "SECRET-123" } },
        { "tool": "redact_mark", "args": { "doc": 1, "find": "SECRET-123" } },
        { "tool": "redact_apply", "args": { "doc": 1 } },
        { "tool": "doc_save", "args": { "doc": 1, "path": "first.pdf", "full": false } },
        { "tool": "doc_save", "args": { "doc": 1, "path": "second.pdf", "full": false } },
        { "tool": "edit_undo", "args": { "doc": 1 } }
    ]);
    std::fs::write(dir.join("steps.json"), steps.to_string()).unwrap();
    let out =
        Command::new(env!("CARGO_BIN_EXE_pdfcraft-cli")).current_dir(&dir).args(["run", "--root", ".", "--script", "steps.json"]).output().unwrap();
    assert!(!out.status.success(), "Undo must be refused after Apply");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("edit_undo") && stderr.contains("nothing to undo"), "{stderr}");
    for name in ["first.pdf", "second.pdf"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert!(!bytes.windows(10).any(|w| w == b"SECRET-123"), "{name} retains the secret");
        let info = pdfcraft_render::inspect(std::sync::Arc::new(bytes), None).unwrap();
        assert!(info.title.is_none(), "{name} retains the title");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
