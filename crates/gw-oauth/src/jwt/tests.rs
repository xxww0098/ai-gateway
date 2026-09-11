use super::{decode_payload, email_and_account};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn token_with_payload(json: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json.as_bytes());
    format!("header.{payload}.sig")
}

#[test]
fn object_payload_round_trips() {
    let token = token_with_payload(r#"{"email":"a@b.test","sub":"user-1"}"#);
    let map = decode_payload(&token).expect("object");
    assert_eq!(map["email"], "a@b.test");
}

#[test]
fn array_or_empty_payload_is_rejected() {
    assert!(decode_payload("").is_none());
    assert!(decode_payload("not-a-jwt").is_none());
    let array = token_with_payload("[1]");
    assert!(decode_payload(&array).is_none());
}

#[test]
fn openai_auth_claim_wins_over_sub() {
    let token = token_with_payload(
        r#"{"email":"a@b.test","sub":"sub-1","https://api.openai.com/auth":{"account_id":"acct-9"}}"#,
    );
    let (email, account) = email_and_account(&token);
    assert_eq!(email, "a@b.test");
    assert_eq!(account, "acct-9");
}
