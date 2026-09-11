//! RFC 8628 poll classification. Networked start/poll live in `grok/tests.rs`.

use super::{PollStep, interpret_poll};
use crate::Error;

#[test]
fn a_successful_token_body_is_tokens_not_pending() {
    let step = interpret_poll(
        200,
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#,
        5,
    )
    .expect("tokens");
    match step {
        PollStep::Tokens(value) => {
            assert_eq!(value["access_token"], "at-1");
        }
        other => panic!("expected tokens, got {other:?}"),
    }
}

#[test]
fn missing_access_token_on_success_status_is_payload() {
    let err = interpret_poll(200, r#"{"token_type":"Bearer"}"#, 5).expect_err("empty");
    assert!(matches!(err, Error::Payload(_)), "{err}");
}

#[test]
fn device_start_rejects_a_body_without_user_code() {
    let err = super::parse_device_start(
        r#"{"device_code":"dev","verification_uri":"https://auth.x.ai/device"}"#,
    )
    .expect_err("missing");
    assert!(matches!(err, Error::Payload(_)), "{err}");
}
