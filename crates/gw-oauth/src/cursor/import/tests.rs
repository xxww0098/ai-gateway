use serde_json::json;

use super::{session_from_json, vscdb_paths, windows_username_from_env};
use crate::Error;

fn jwt(payload: &str) -> String {
    use base64::Engine as _;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("{header}.{body}.x")
}

#[test]
fn missing_access_is_invalid_import() {
    let err = session_from_json(&json!({"refreshToken": "r"})).expect_err("missing");
    assert!(matches!(err, Error::InvalidImport(_)));
}

#[test]
fn snake_and_camel_case_tokens_are_accepted() {
    let access = jwt(r#"{"email":"a@x","exp":9999999999}"#);
    let camel = session_from_json(&json!({
        "accessToken": access,
        "refreshToken": "rt-camel",
    }))
    .expect("camel");
    assert_eq!(camel.refresh_token, "rt-camel");
    let snake = session_from_json(&json!({
        "access_token": access,
        "refresh_token": "rt-snake",
    }))
    .expect("snake");
    assert_eq!(snake.refresh_token, "rt-snake");
    let nested = session_from_json(&json!({
        "session": {"access": access, "refresh": "rt-nested", "source": "env"}
    }))
    .expect("nested");
    assert_eq!(nested.source, "env");
}

#[test]
fn wsl_username_does_not_walk_other_users() {
    assert_eq!(
        windows_username_from_env(&[("USERPROFILE", r"C:\Users\alice"), ("USERNAME", "alice")])
            .as_deref(),
        Some("alice")
    );
    assert_eq!(
        windows_username_from_env(&[("USERPROFILE", r"C:\Users\Public"), ("USERNAME", "alice")])
            .as_deref(),
        Some("alice")
    );
    assert!(windows_username_from_env(&[("USERNAME", "Public")]).is_none());
    assert!(windows_username_from_env(&[("USERNAME", "Default")]).is_none());
    let paths = vscdb_paths(
        "linux",
        "/home/alice",
        &[
            ("WSL_DISTRO_NAME", "Ubuntu"),
            ("USERPROFILE", r"C:\Users\alice"),
            ("USERNAME", "alice"),
        ],
    );
    assert!(
        paths
            .iter()
            .any(|p| p.contains("/mnt/c/Users/alice/AppData/Roaming/Cursor/"))
    );
    assert!(!paths.iter().any(|p| p.contains("/mnt/c/Users/Public/")
        || p.contains("/mnt/c/Users/Default/")
        || p.contains("/mnt/c/Users/bob/")));
    assert_eq!(
        paths.iter().filter(|p| p.contains("/mnt/c/Users/")).count(),
        1
    );
}
