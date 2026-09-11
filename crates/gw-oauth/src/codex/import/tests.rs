use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use super::{import_from_paths, search_paths, session_from_json};
use crate::Error;

fn unsigned_jwt(payload: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    format!("{header}.{body}.x")
}

fn id_token(account: &str, email: &str, plan: &str) -> String {
    unsigned_jwt(&json!({
        "email": email,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account,
            "chatgpt_plan_type": plan,
        },
    }))
}

fn access_token() -> String {
    unsigned_jwt(&json!({"exp": chrono::Utc::now().timestamp() + 3600}))
}

#[test]
fn cli_token_file_becomes_a_session() {
    let email = "plus@example.com";
    let account = "org-1";
    let plan = "plus";
    let session = session_from_json(&json!({
        "tokens": {
            "access_token": access_token(),
            "refresh_token": "refresh",
            "expires_in": 3600,
            "id_token": id_token(account, email, plan),
        },
    }))
    .expect("session");
    assert_eq!(session.account, email);
    assert_eq!(session.plan_type, plan);
    assert_eq!(session.extra["account_id"], account);
    assert!(session.expires_at_ms > chrono::Utc::now().timestamp_millis());
}

#[test]
fn hermes_openai_codex_provider_is_accepted() {
    let account = "org-2";
    let session = session_from_json(&json!({
        "providers": {
            "openai-codex": {
                "tokens": {
                    "access_token": access_token(),
                    "refresh_token": "hermes-refresh",
                    "expires_in": 2400,
                    "id_token": id_token(account, "h@example.com", "pro"),
                },
            },
        },
    }))
    .expect("session");
    assert_eq!(session.refresh_token, "hermes-refresh");
    assert_eq!(session.extra["account_id"], account);
}

#[test]
fn missing_tokens_are_invalid_import() {
    let err = session_from_json(&json!({"tokens": {"refresh_token": "only"}})).unwrap_err();
    assert!(matches!(err, Error::InvalidImport(_)));
}

#[test]
fn search_paths_end_with_codex_then_hermes_auth_json() {
    let paths = search_paths();
    assert!(
        !paths.is_empty(),
        "HOME/USERPROFILE must resolve so Codex import has search paths"
    );
    let rendered: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(rendered[0].ends_with(".codex/auth.json"));
    assert!(rendered[1].ends_with(".hermes/auth.json"));
}

#[test]
fn import_from_paths_reads_the_first_usable_file() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gw-oauth-codex-import-{stamp}"));
    fs::create_dir_all(&dir).expect("dir");
    let missing = dir.join("missing.json");
    let path = dir.join(".codex-auth.json");
    let account = "org-file";
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "tokens": {
                "access_token": access_token(),
                "refresh_token": "from-file",
                "expires_in": 1800,
                "id_token": id_token(account, "file@example.com", "plus"),
            },
        }))
        .expect("json"),
    )
    .expect("write");
    let session = import_from_paths(&[missing, path.clone()]).expect("import");
    assert_eq!(session.refresh_token, "from-file");
    assert_eq!(session.extra["account_id"], account);
    assert!(session.source.contains(".codex-auth.json"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn import_from_paths_lists_tried_files_when_nothing_matches() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let a = std::env::temp_dir().join(format!("gw-oauth-codex-a-{stamp}.json"));
    let b = std::env::temp_dir().join(format!("gw-oauth-codex-b-{stamp}.json"));
    let err = import_from_paths(&[a.clone(), b.clone()]).unwrap_err();
    match err {
        Error::InvalidImport(message) => {
            assert!(message.contains(&a.display().to_string()));
            assert!(message.contains(&b.display().to_string()));
        }
        other => panic!("expected invalid import, got {other}"),
    }
}
