//! Crate-level invariants: no inference HTTP, no cross-family cache imports.

use std::fs;
use std::path::{Path, PathBuf};

fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("src") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    walk(&src, &mut out);
    out
}

fn relative(path: &Path) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    path.strip_prefix(&src)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn scannable(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") { "" } else { line }
}

/// A hop crate that can `.send()` will grow a second inference path.
#[test]
fn sources_do_not_send() {
    for path in sources() {
        let rel = relative(&path);
        if rel.ends_with("tests.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read");
        for (i, line) in text.lines().enumerate() {
            let line = scannable(line);
            assert!(
                !line.contains(".send("),
                "{}:{} sends — hop planning has no sockets",
                rel,
                i + 1
            );
            assert!(
                !line.contains("reqwest"),
                "{}:{} names reqwest — keep HTTP out of this crate",
                rel,
                i + 1
            );
        }
    }
}

/// Family modules must not import another family's cache helpers.
#[test]
fn family_modules_do_not_import_each_other() {
    let pairs = [
        ("codex.rs", "crate::grok"),
        ("codex.rs", "crate::kiro"),
        ("grok.rs", "crate::codex"),
        ("grok.rs", "crate::kiro"),
        ("kiro.rs", "crate::codex"),
        ("kiro.rs", "crate::grok"),
    ];
    for path in sources() {
        let rel = relative(&path);
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let text = fs::read_to_string(&path).expect("read");
        for (owner, forbidden) in pairs {
            if file == owner || rel.starts_with(&format!("{}/", owner.trim_end_matches(".rs"))) {
                assert!(
                    !text.contains(forbidden),
                    "{rel} imports {forbidden} — cache helpers stay in-family"
                );
            }
        }
    }
}

/// The crate manifest must not grow an HTTP client or an RNG by accident.
#[test]
fn manifest_has_no_http_client() {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("manifest");
    for name in ["reqwest", "hyper", "tokio", "uuid"] {
        assert!(
            !manifest.contains(name),
            "gw-oauth-hops Cargo.toml must not depend on {name}"
        );
    }
}
