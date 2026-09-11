use serde_json::json;

use super::{ImportKind, hydrate_sso_token, imported_session, parse_import_text};
use crate::kiro::session::{extra_str, infer_auth_method};

fn long_rt() -> String {
    format!("rt_{}", "x".repeat(120))
}

#[test]
fn json_without_access_token_is_rejected() {
    let err =
        imported_session(&json!({"token": {"refresh_token": "rt"}})).expect_err("missing access");
    let msg = err.to_string();
    assert!(msg.contains("access_token"), "{msg}");
}

#[test]
fn json_object_with_access_imports() {
    let rt = long_rt();
    let session = imported_session(&json!({
        "token": {
            "accessToken": "at",
            "refreshToken": rt,
            "email": "a@x",
            "authMethod": "social",
        }
    }))
    .expect("import");
    assert_eq!(session.access_token, "at");
    assert_eq!(session.account, "a@x");
    assert_eq!(extra_str(&session, "auth_method"), "social");
}

#[test]
fn kami_csv_json_and_ksk_parse() {
    let rt = long_rt();
    let kami = parse_import_text(&format!(
        "a@x----no_password----{rt}----cid----sec----BuilderId"
    ));
    assert_eq!(kami.kind, ImportKind::Kami);
    assert_eq!(kami.sessions.len(), 1);
    assert_eq!(extra_str(&kami.sessions[0], "auth_method"), "idc");
    assert_eq!(kami.sessions[0].account, "a@x");
    assert!(!kami.sessions[0].access_token.is_empty());

    let compact = parse_import_text(&serde_json::to_string(&json!([
        {"email": "gh@x", "refreshToken": rt, "provider": "Github", "accessToken": "at-gh"},
        {"email": "b@x", "refreshToken": rt, "provider": "BuilderId", "clientId": "cid", "clientSecret": "sec", "accessToken": "at-b"}
    ]))
    .expect("json"));
    assert_eq!(compact.kind, ImportKind::Json);
    assert_eq!(compact.sessions.len(), 2);
    assert_eq!(extra_str(&compact.sessions[0], "auth_method"), "social");
    assert_eq!(extra_str(&compact.sessions[1], "auth_method"), "idc");

    let csv = parse_import_text(&format!(
        "邮箱,refreshToken,登录方式,clientId,clientSecret,accessToken\nb@x,{rt},BuilderId,cid,sec,at-csv"
    ));
    assert_eq!(csv.kind, ImportKind::Csv);
    assert_eq!(csv.sessions.len(), 1);
    assert_eq!(extra_str(&csv.sessions[0], "auth_method"), "idc");

    let key = parse_import_text("ksk_live_example1");
    assert_eq!(key.kind, ImportKind::Keys);
    assert_eq!(key.sessions.len(), 1);
    assert_eq!(extra_str(&key.sessions[0], "auth_method"), "api_key");
}

#[test]
fn infer_method_from_provider_without_auth_method() {
    let rt = long_rt();
    assert_eq!(
        infer_auth_method(
            &json!({"provider": "BuilderId", "refreshToken": rt, "clientId": "cid", "clientSecret": "sec"})
        ),
        "idc"
    );
    assert_eq!(
        infer_auth_method(&json!({"provider": "Github", "refreshToken": rt})),
        "social"
    );
    assert_eq!(
        infer_auth_method(&json!({"kiroApiKey": "ksk_live_example1"})),
        "api_key"
    );
}

#[test]
fn hydrate_fills_missing_oidc_client() {
    let merged = hydrate_sso_token(
        &json!({"refreshToken": long_rt(), "authMethod": "IdC"}),
        Some(&json!({"clientId": "x", "clientSecret": "y"})),
    );
    assert_eq!(merged["clientId"], "x");
    assert_eq!(merged["clientSecret"], "y");
}
