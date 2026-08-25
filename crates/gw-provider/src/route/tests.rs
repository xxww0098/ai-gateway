//! The guard that keeps this crate out of the inference HTTP business.
//!
//! Deleting `Provider::execute` once is easy. Keeping it deleted is the part
//! that needs a test: the next executor to be added will have a `reqwest`
//! example within reach, and a second inference path that *works* is a second
//! inference path that gets used — with its own header handling, its own
//! error-to-status mapping, and its own idea of what a mid-stream failure
//! looks like.
//!
//! So this walks `src/**` and fails on the shapes that would mean the split
//! came undone. It reads the source rather than the type system because the
//! thing being forbidden is a *capability* (sending), not a signature.

use std::path::{Path, PathBuf};

/// The one module allowed to send: credential refresh. See `oauth.rs` for why
/// that is a different kind of traffic.
const SENDING_ALLOWLIST: &[&str] = &["oauth.rs"];

/// This file, which necessarily *names* every forbidden shape.
///
/// Skipped by path rather than by name so a real `route/tests.rs` in some other
/// directory would still be scanned.
const GUARD_SELF: &str = "route/tests.rs";

/// Every `.rs` file under this crate's `src/`.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|err| panic!("reading {}: {err}", dir.display()));
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
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
    assert!(!out.is_empty(), "found no sources under {}", src.display());
    out.sort();
    out
}

/// Whether `path` is this guard itself.
fn is_guard_self(path: &Path) -> bool {
    relative(path).replace('\\', "/") == GUARD_SELF
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// The scannable part of a line: everything, unless the whole line is a
/// comment.
///
/// Deliberately *not* "cut at the first `//`". That is what a first attempt
/// does, and it is blind: `client.get("https://x").send()` contains `//`
/// inside a URL, so cutting there throws the `.send()` away and the guard
/// reports clean. Whole-line comments are skipped; a trailing comment that
/// names a forbidden shape will trip the guard, which is the safe direction —
/// a noisy guard gets fixed, a blind one does not get noticed.
fn scannable(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    line
}

/// `<crate>/src/`-relative path, for a failure message that can be acted on.
fn relative(path: &Path) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    path.strip_prefix(&src)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn no_source_outside_the_oauth_module_sends_an_http_request() {
    // `.send()` on a `RequestBuilder` is the single expression that turns a
    // built request into traffic. Nothing else in this crate may contain it.
    let mut offenders = Vec::new();
    for path in sources() {
        if SENDING_ALLOWLIST.contains(&file_name(&path).as_str()) || is_guard_self(&path) {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in body.lines().enumerate() {
            if scannable(line).contains(".send()") {
                offenders.push(format!("{}:{}", relative(&path), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "inference HTTP must leave through gw-relay, not this crate. \
         Move credential-refresh traffic into oauth.rs; everything else belongs \
         in a RoutePlan. Offending sites: {offenders:?}",
    );
}

#[test]
fn the_executor_api_is_gone_rather_than_merely_unused() {
    // A dormant `execute` / `execute_stream` is a dual path waiting for a
    // caller. These names must not reappear anywhere in the crate.
    const FORBIDDEN: &[&str] = &[
        "async fn execute(",
        "async fn execute_stream",
        "fn execute_stream",
    ];
    let mut offenders = Vec::new();
    for path in sources() {
        if is_guard_self(&path) {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in body.lines().enumerate() {
            for needle in FORBIDDEN {
                if scannable(line).contains(needle) {
                    offenders.push(format!("{}:{} ({needle})", relative(&path), index + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the provider executor API was reintroduced: {offenders:?}",
    );
}

#[test]
fn no_source_outside_the_oauth_module_builds_an_http_client() {
    // A client that exists is a client something will eventually send with.
    let mut offenders = Vec::new();
    for path in sources() {
        if SENDING_ALLOWLIST.contains(&file_name(&path).as_str()) || is_guard_self(&path) {
            continue;
        }
        // `common.rs` owns the one pooled client the refresh path borrows.
        if file_name(&path) == "common.rs" {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in body.lines().enumerate() {
            let code = scannable(line);
            if code.contains("reqwest::Client::builder") || code.contains("reqwest::Client::new") {
                offenders.push(format!("{}:{}", relative(&path), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "HTTP clients belong to gw-relay (inference) or common.rs (refresh): {offenders:?}",
    );
}

#[test]
fn the_guard_would_notice_a_send_that_came_back() {
    // Guards that cannot fail are decoration. This drives the same `scannable`
    // the three tests above use, so a refactor that blinds the matcher shows up
    // here rather than as three silently-passing tests.
    for planted in [
        "    let response = builder.send().await?;",
        // The line that defeated the first version of this guard: `//` inside
        // a URL made a naive "cut at the first //" throw the send away.
        r#"        let _ = client.get("https://api.example.com").send().await;"#,
    ] {
        assert!(
            scannable(planted).contains(".send()"),
            "the guard went blind to {planted:?}"
        );
    }

    // A whole-line comment is not code.
    assert!(!scannable("    // the old path called .send() here").contains(".send()"));
    assert!(!scannable("//! `.send()` lives in oauth.rs").contains(".send()"));
}

// ------------------------------------------------------------------ RoutePlan

use super::*;

fn plan(endpoint: &str) -> RoutePlan {
    RoutePlan {
        provider: "openai",
        endpoint: Url::parse(endpoint).expect("a valid endpoint"),
        credential: gw_relay::Credential::Bearer("token".to_owned()),
        headers: HeaderMap::new(),
        body: None,
        timeouts: gw_relay::RelayTimeouts::default(),
        dialect: gw_relay::UpstreamDialect::OpenAiChat,
    }
}

#[test]
fn splitting_and_rejoining_an_endpoint_is_the_identity() {
    // The relay assembles `origin + target` by string concatenation, so the
    // split has to be exactly reversible — including a query it must not
    // decode and re-encode.
    for endpoint in [
        "https://api.example.com/v1/chat/completions",
        "https://api.example.com/v1/responses?stream=true",
        "https://api.example.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse",
        "https://api.example.com/v1/messages?tag=a%20b&tag=c",
        "https://example.com:8443/prefix/v1/messages",
    ] {
        let (origin, target) = plan(endpoint).split().expect("splits");
        let rejoined = format!(
            "{}{}",
            origin.as_str().trim_end_matches('/'),
            target.as_str()
        );
        assert_eq!(rejoined, endpoint, "split of {endpoint} did not round-trip");
    }
}

#[test]
fn a_percent_encoded_query_survives_the_split_byte_for_byte() {
    // The double-encoding defect: decode-then-re-encode turns `?tag=a%20b`
    // into `?tag=a%2520b`.
    let (_, target) = plan("https://api.example.com/v1/messages?tag=a%20b")
        .split()
        .expect("splits");
    assert!(target.as_str().ends_with("?tag=a%20b"));
}

#[test]
fn splicing_concatenates_both_halves_and_nothing_else() {
    let payload = bytes::Bytes::from_static(br#"{"model":"gpt-4o","stream":true}"#);
    let spliced =
        crate::common::ensure_include_usage(&payload, gw_relay::Surface::OpenAiCompletions)
            .expect("this fixture splices");
    let expected_len = spliced.len();
    let joined = RoutePlan::splice(Some(spliced)).expect("some");
    assert_eq!(joined.len(), expected_len);
    assert!(RoutePlan::splice(None).is_none());
}
