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

/// ChatGPT subscription OAuth is not an OpenAI Platform API key: it lacks
/// `api.model.read`, so `api.openai.com/v1/models` returns 403. Codex CLI
/// lists models on the ChatGPT backend, and `client_version` is required.
const CODEX_BACKEND_MODELS: &str = "https://chatgpt.com/backend-api/codex/models";
const CODEX_VERSION_ENV: &str = "AGW_CODEX_CLI_VERSION";
const CODEX_CLI_FALLBACK_VERSION: &str = "0.149.0";

fn codex_cli_version() -> String {
    if let Ok(raw) = std::env::var(CODEX_VERSION_ENV) {
        let trimmed = raw.trim().trim_start_matches('v');
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    discover_codex_cli_version().unwrap_or_else(|| CODEX_CLI_FALLBACK_VERSION.to_owned())
}

fn discover_codex_cli_version() -> Option<String> {
    let output = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_dotted_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_dotted_version(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let token = token
            .trim_start_matches('v')
            .trim_end_matches(|c: char| !c.is_ascii_digit());
        let parts: Vec<&str> = token.split('.').collect();
        (parts.len() >= 3
            && parts
                .iter()
                .take(3)
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())))
        .then(|| parts[..3].join("."))
    })
}

fn codex_backend_models_url(client_version: &str) -> String {
    let version = client_version.trim();
    assert!(
        !version.is_empty(),
        "Codex backend /models requires client_version"
    );
    let mut url = url::Url::parse(CODEX_BACKEND_MODELS).expect("static Codex backend models URL");
    url.query_pairs_mut().append_pair("client_version", version);
    url.to_string()
}

fn looks_like_codex_catalog(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        return false;
    };
    models.iter().any(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| !slug.is_empty())
            || model
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
    })
}

/// The probe must hit ChatGPT's Codex catalog, not Platform `/v1/models`.
#[test]
fn codex_models_probe_targets_chatgpt_backend_not_platform_api() {
    let version = "1.2.3";
    let url = url::Url::parse(&codex_backend_models_url(version)).expect("probe url");
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("chatgpt.com"));
    let path = url.path();
    assert!(
        path.contains("backend-api") && path.contains("codex") && path.ends_with("/models"),
        "{path}"
    );
    let query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert!(
        query
            .iter()
            .any(|(k, v)| k == "client_version" && v == version),
        "{query:?}"
    );
    assert_ne!(url.host_str(), Some("api.openai.com"));
    assert!(!path.contains("/v1/"));
}

/// Codex catalog is `{models:[{slug}]}`. Platform `{data:[{id}]}` is the wrong API.
#[test]
fn codex_catalog_accepts_slug_list_and_rejects_platform_data_list() {
    assert!(looks_like_codex_catalog(
        r#"{"models":[{"slug":"gpt-test","display_name":"gpt-test"}]}"#
    ));
    assert!(looks_like_codex_catalog(
        r#"{"models":[{"id":"gpt-test"}]}"#
    ));
    assert!(!looks_like_codex_catalog(
        r#"{"object":"list","data":[{"id":"gpt-4","object":"model"}]}"#
    ));
    assert!(!looks_like_codex_catalog(r#"{"models":[]}"#));
    assert!(!looks_like_codex_catalog("not-json"));
}

#[test]
fn dotted_cli_version_is_taken_from_codex_version_text() {
    assert_eq!(
        parse_dotted_version("codex-cli 0.42.0"),
        Some("0.42.0".to_owned())
    );
    assert_eq!(parse_dotted_version("not a version"), None);
}

#[tokio::test]
#[ignore = "REAL_API=1 plus ~/.codex/auth.json"]
async fn real_codex_models_lists_when_local_oauth_exists() {
    require_real_api();
    let cred = cred_or_panic("codex");
    let version = codex_cli_version();
    let url = codex_backend_models_url(&version);
    let authorization = format!("Bearer {}", cred.access_token);
    let user_agent = format!("codex-cli/{version}");
    let (status, body) = crate::oauth::get_text(
        &url,
        &[
            ("authorization", authorization.as_str()),
            ("user-agent", user_agent.as_str()),
        ],
    )
    .await
    .unwrap_or_else(|err| panic!("codex models request failed: {err}"));
    assert!(
        (200..300).contains(&status),
        "codex GET chatgpt.com/backend-api/codex/models returned {status}: {body}. \
         ChatGPT OAuth is not a Platform API key (missing api.model.read). \
         If 401, refresh the Codex CLI login and retry."
    );
    assert!(
        looks_like_codex_catalog(&body),
        "codex models response was not {{models:[{{slug}}]}}: {body}"
    );
}

#[tokio::test]
#[ignore = "REAL_API=1 plus ~/.claude/.credentials.json"]
async fn real_claude_oauth_stays_fail_closed_without_chrome_tls() {
    require_real_api();
    let cred = cred_or_panic("claude");
    crate::claude::fingerprint::assert_oauth_http_fingerprint(
        &crate::claude::fingerprint::probe_headers(),
    );
    assert!(
        !crate::claude::fingerprint::chrome_tls_ready(),
        "Chrome TLS flipped on without a ClientHello"
    );
    let err = crate::claude::fingerprint::refuse_unverified_send()
        .expect_err("must not call Anthropic over rustls");
    assert!(
        err.to_string().contains("refused"),
        "OAuth send gate opened: {err}"
    );
    let _ = cred.access_token.len();
    if let Some(path) = crate::claude::fingerprint::capture_path()
        && path.is_file()
    {
        let raw = std::fs::read(&path).unwrap_or_else(|err| {
            panic!(
                "reading {}: {err}. See docs/claude-fingerprint.md",
                path.display()
            )
        });
        crate::claude::fingerprint::compare_capture(&raw).unwrap_or_else(|err| {
            panic!(
                "capture {} does not match cloak invariants: {err}",
                path.display()
            )
        });
    }
}
