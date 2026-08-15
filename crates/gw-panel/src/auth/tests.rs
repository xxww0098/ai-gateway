//! Unit tests for the panel auth extractors.
//!
//! Everything that needs Postgres lives in the integration suite; what is
//! testable here is the decision logic that sits *around* the queries — which
//! credential path a token takes, and how a `users` row that 既有 schema 留成 NULL is
//! normalised before the admin gate sees it.
//!
//! Rule 2.11: no assertion restates a literal from `super`. The role tests
//! deliberately cross-check two *independently declared* things — this module's
//! normalisation and `crate::AuthUser::is_admin`'s comparison — which is a real
//! consistency property, not a tautology.

use super::*;
use crate::AuthUser;

fn user_with_role(role: &str) -> AuthUser {
    AuthUser {
        user_id: 1,
        email: String::new(),
        role: role.to_owned(),
        api_key_id: None,
        group_id: None,
        rate_multiplier: 1.0,
    }
}

#[test]
fn api_key_tokens_take_the_api_key_path() {
    let key = gw_authcore::new_api_key().expect("key generation");
    assert!(is_api_key_token(&key));
}

#[test]
fn jwt_shaped_tokens_take_the_jwt_path() {
    // A real HS256 token: three base64url segments. Nothing about it may be
    // mistaken for an API key, or a valid JWT would be hashed and looked up in
    // `api_keys` and rejected.
    let token =
        gw_authcore::generate_jwt(1, "user@example.test", "secret", 1).expect("token generation");
    assert!(!is_api_key_token(&token));
}

#[test]
fn the_api_key_prefix_is_not_matched_mid_token() {
    // `starts_with`, not `contains` — a JWT whose payload happens to encode the
    // prefix must still go down the JWT path.
    assert!(!is_api_key_token(&format!(
        "x{}",
        gw_authcore::API_KEY_PREFIX
    )));
}

#[test]
fn a_null_users_row_reads_as_the_zero_value() {
    // 旧实现把 NULL 列填成零值；anything else would make a
    // legacy row undecodable and 401 a legitimate user.
    let identity = Identity::default();
    assert_eq!(identity.status(), "");
    assert_eq!(identity.role(), "");
    assert_eq!(identity.email(), "");
}

#[test]
fn a_missing_status_is_not_active() {
    // The "no such user" sentinel must never collide with the active status, or
    // a deleted user's cached entry would authenticate.
    assert_ne!(STATUS_MISSING, STATUS_ACTIVE);
    assert_ne!(Identity::default().status(), STATUS_ACTIVE);
}

#[test]
fn role_normalisation_agrees_with_the_admin_gate() {
    // `AuthUser::is_admin` is declared in the crate root and compares exactly;
    // this module is what feeds it. If either side changes independently, an
    // admin stops being able to reach admin routes — so pin the pairing.
    for raw in ["admin", "ADMIN", "  Admin  ", "\tadmin\n"] {
        let identity = Identity {
            role: Some(raw.to_owned()),
            ..Identity::default()
        };
        assert!(
            user_with_role(&identity.role()).is_admin(),
            "role {raw:?} should normalise to an admin"
        );
    }
}

#[test]
fn near_miss_roles_are_not_admin() {
    for raw in ["administrator", "user", "adm in", "", "superadmin"] {
        let identity = Identity {
            role: Some(raw.to_owned()),
            ..Identity::default()
        };
        assert!(
            !user_with_role(&identity.role()).is_admin(),
            "role {raw:?} must not pass the admin gate"
        );
    }
}

#[test]
fn cached_credentials_do_not_outlive_the_status_cache_ceiling() {
    // A key cached for longer than a status entry could keep serving a
    // suspended user past the window the status recheck is supposed to close.
    assert!(API_KEY_TTL <= gw_infra::cache::MAX_USER_STATUS_TTL);
    assert!(USER_STATUS_TTL <= gw_infra::cache::MAX_USER_STATUS_TTL);
}
