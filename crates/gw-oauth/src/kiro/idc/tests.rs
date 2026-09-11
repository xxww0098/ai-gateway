use super::{device_authorization_body, register_body};
use crate::kiro::{BUILDER_ID_START_URL, DEVICE_GRANT};

#[test]
fn register_body_is_json_camel_case_public_client() {
    let body = register_body(BUILDER_ID_START_URL);
    assert_eq!(body["clientType"], "public");
    assert_eq!(body["issuerUrl"], BUILDER_ID_START_URL);
    assert!(
        body["grantTypes"]
            .as_array()
            .expect("grants")
            .iter()
            .any(|g| g.as_str() == Some(DEVICE_GRANT))
    );
    assert!(body.get("client_id").is_none());
    let device = device_authorization_body("cid", "sec", BUILDER_ID_START_URL);
    assert_eq!(device["clientId"], "cid");
    assert_eq!(device["startUrl"], BUILDER_ID_START_URL);
}
