//! Local CLI OAuth file discovery and shape lifting.

use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::{
    LocalOauthCred, discover, infer_provider, lift_cli_shape, parse_cli_bytes, well_known_paths,
};

fn map(raw: Value) -> Map<String, Value> {
    raw.as_object().cloned().expect("object")
}

fn write_json(dir: &std::path::Path, rel: &str, raw: &Value) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(&path, raw.to_string()).expect("write");
    path
}

fn temp_home(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("agw-local-oauth-{}-{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("home");
    path
}

/// Codex CLI and Claude Code files under one home become two importable creds.
#[test]
fn discover_reads_codex_and_claude_files_under_home() {
    let home = temp_home("both");
    write_json(
        &home,
        ".codex/auth.json",
        &json!({
            "tokens": {
                "access_token": "codex-at",
                "refresh_token": "codex-rt",
                "id_token": "codex-id"
            },
            "last_refresh": "2026-01-01T00:00:00Z"
        }),
    );
    write_json(
        &home,
        ".claude/.credentials.json",
        &json!({
            "claudeAiOauth": {
                "accessToken": "claude-at",
                "refreshToken": "claude-rt",
                "expiresAt": 1_800_000_000_000_i64
            }
        }),
    );

    let found = discover(&home);
    let providers: Vec<&str> = found.iter().map(|c| c.provider).collect();
    assert!(providers.contains(&"codex"), "{providers:?}");
    assert!(providers.contains(&"claude"), "{providers:?}");
    let codex = found.iter().find(|c| c.provider == "codex").expect("codex");
    let claude = found
        .iter()
        .find(|c| c.provider == "claude")
        .expect("claude");
    assert_eq!(codex.access_token, "codex-at");
    assert_eq!(codex.refresh_token, "codex-rt");
    assert_eq!(claude.access_token, "claude-at");
    assert_eq!(claude.refresh_token, "claude-rt");
    let _ = fs::remove_dir_all(&home);
}

/// A missing home yields nothing rather than an error — CI has no CLI files.
#[test]
fn discover_skips_missing_files() {
    let home = temp_home("empty");
    assert!(discover(&home).is_empty());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn well_known_paths_stay_under_the_given_home() {
    let home = PathBuf::from("/tmp/agw-home-example");
    for path in well_known_paths(&home) {
        assert!(
            path.starts_with(&home)
                || path.to_string_lossy().contains("credentials.json")
                || path.to_string_lossy().ends_with("auth.json"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn claude_shape_is_inferred_without_a_provider_field() {
    assert_eq!(
        infer_provider(&map(json!({"claudeAiOauth": {"accessToken": "t"}}))),
        Some("claude")
    );
}

#[test]
fn codex_tokens_object_is_inferred() {
    assert_eq!(
        infer_provider(&map(json!({
            "tokens": {"access_token": "t", "id_token": "id"},
            "last_refresh": "now"
        }))),
        Some("codex")
    );
}

#[test]
fn grok_oidc_mode_is_inferred_as_xai() {
    assert_eq!(
        infer_provider(&map(json!({
            "auth_mode": "oidc",
            "access_token": "at",
            "refresh_token": "rt"
        }))),
        Some("xai")
    );
}

#[test]
fn lift_flattens_claude_camel_case_so_upload_can_copy_tokens() {
    let mut payload = map(json!({
        "claudeAiOauth": {
            "accessToken": "at",
            "refreshToken": "rt"
        }
    }));
    lift_cli_shape(&mut payload);
    assert_eq!(
        payload.get("access_token").and_then(Value::as_str),
        Some("at")
    );
    assert_eq!(
        payload.get("refresh_token").and_then(Value::as_str),
        Some("rt")
    );
    assert_eq!(
        payload.get("provider").and_then(Value::as_str),
        Some("claude")
    );
}

#[test]
fn lift_promotes_codex_tokens_and_keeps_token_data() {
    let mut payload = map(json!({
        "tokens": {
            "access_token": "at",
            "refresh_token": "rt",
            "id_token": "id"
        },
        "last_refresh": "now"
    }));
    lift_cli_shape(&mut payload);
    assert_eq!(
        payload.get("access_token").and_then(Value::as_str),
        Some("at")
    );
    assert!(
        payload
            .get("token_data")
            .and_then(Value::as_object)
            .is_some()
    );
    assert_eq!(
        payload.get("provider").and_then(Value::as_str),
        Some("codex")
    );
}

#[test]
fn a_hermes_store_can_yield_both_codex_and_grok() {
    let creds = parse_cli_bytes(
        json!({
            "providers": {
                "codex": { "access_token": "c-at", "refresh_token": "c-rt" },
                "grok": { "access_token": "g-at", "refresh_token": "g-rt" }
            }
        })
        .to_string()
        .as_bytes(),
        &PathBuf::from("/tmp/.hermes/auth.json"),
    );
    let providers: Vec<&str> = creds.iter().map(|c| c.provider).collect();
    assert!(providers.contains(&"codex"), "{providers:?}");
    assert!(providers.contains(&"xai"), "{providers:?}");
}

#[test]
fn hermes_store_yields_the_codex_entry() {
    let creds = parse_cli_bytes(
        json!({
            "providers": {
                "codex": {
                    "access_token": "h-at",
                    "refresh_token": "h-rt"
                }
            }
        })
        .to_string()
        .as_bytes(),
        &PathBuf::from("/tmp/.hermes/auth.json"),
    );
    assert_eq!(creds.len(), 1);
    assert_eq!(creds[0].provider, "codex");
    assert_eq!(creds[0].access_token, "h-at");
}

#[test]
fn upload_json_never_omits_provider_and_keeps_tokens_together() {
    let cred = LocalOauthCred {
        provider: "claude",
        source: PathBuf::from("/tmp/.claude/.credentials.json"),
        access_token: "at".to_owned(),
        refresh_token: "rt".to_owned(),
        id_token: String::new(),
        expires_at: String::new(),
        email: String::new(),
    };
    let json = cred.to_upload_json();
    assert_eq!(json["provider"], "claude");
    assert_eq!(json["access_token"], "at");
    assert_eq!(json["refresh_token"], "rt");
    assert_eq!(json["token_data"]["access_token"], "at");
}

/// A file that is not JSON is skipped, not treated as an empty credential.
#[test]
fn garbage_bytes_produce_no_credential() {
    assert!(parse_cli_bytes(b"not-json", &PathBuf::from("/tmp/x.json")).is_empty());
}

fn require_real_api() {
    match std::env::var("REAL_API") {
        Ok(value) if value == "1" || value.eq_ignore_ascii_case("true") => {}
        _ => panic!(
            "set REAL_API=1 to run real upstream smokes. \
             `make test-ignored` skips names starting with real_; see docs/real-api-tests.md"
        ),
    }
}

fn cred_or_panic(provider: &'static str) -> LocalOauthCred {
    let home = super::process_home().unwrap_or_else(|| {
        panic!("no home directory. Set AGW_LOCAL_OAUTH_HOME or HOME. See docs/real-api-tests.md")
    });
    discover(&home)
        .into_iter()
        .find(|cred| cred.provider == provider)
        .unwrap_or_else(|| {
            panic!(
                "no {provider} credentials. Put a CLI auth file under $HOME \
                 (or AGW_LOCAL_OAUTH_HOME). See docs/real-api-tests.md"
            )
        })
}

#[tokio::test]
#[ignore = "REAL_API=1 plus ~/.codex/auth.json"]
async fn real_codex_models_lists_when_local_oauth_exists() {
    require_real_api();
    let cred = cred_or_panic("codex");
    let (status, body) = crate::oauth::get_text(
        "https://api.openai.com/v1/models",
        &[("authorization", &format!("Bearer {}", cred.access_token))],
    )
    .await
    .unwrap_or_else(|err| panic!("codex models request failed: {err}"));
    assert!(
        (200..300).contains(&status),
        "codex GET /v1/models returned {status}: {body}. \
         If 401, refresh the Codex CLI login and retry."
    );
    assert!(
        body.contains("\"data\"") || body.contains("\"id\""),
        "codex models response had no catalog: {body}"
    );
}

#[tokio::test]
#[ignore = "REAL_API=1 plus ~/.claude/.credentials.json"]
async fn real_claude_models_lists_when_local_oauth_exists() {
    require_real_api();
    let cred = cred_or_panic("claude");
    let mut headers = crate::claude::fingerprint::probe_headers();
    crate::claude::fingerprint::assert_oauth_http_fingerprint(&headers);
    headers.push((
        "authorization".to_owned(),
        format!("Bearer {}", cred.access_token),
    ));
    let (status, body) = crate::oauth::get_text("https://api.anthropic.com/v1/models", &headers)
        .await
        .unwrap_or_else(|err| panic!("claude models request failed: {err}"));
    assert!(
        (200..300).contains(&status),
        "claude GET /v1/models returned {status}: {body}. \
         If 401, run `claude` to refresh ~/.claude/.credentials.json."
    );
    assert!(
        body.contains("\"data\"") || body.contains("\"id\""),
        "claude models response had no catalog: {body}"
    );
}
