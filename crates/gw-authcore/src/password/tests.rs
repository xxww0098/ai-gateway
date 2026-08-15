//! `REF_HASH` was produced by an independent bcrypt implementation at default
//! cost and stands in for the rows already sitting in `users.password_hash`.

use super::{hash_password, verify_password};

const REF_PASSWORD: &str = "correct-horse-battery-staple";
const REF_HASH: &str = "$2a$10$qithO4mzpO3if6uXVXS1OOXTL4OqQfzOsoyiV8o5HogHVwLzvrY7u";

/// `$2<minor>$<cost>$…` → cost.
fn cost_of(hash: &str) -> &str {
    hash.split('$').nth(2).expect("bcrypt hashes carry a cost")
}

#[test]
fn passwords_hashed_externally_still_log_in() {
    assert!(
        verify_password(REF_PASSWORD, REF_HASH).expect("the hash is well-formed"),
        "existing accounts must survive the rewrite"
    );
}

#[test]
fn a_wrong_password_does_not_match_an_existing_hash() {
    assert!(!verify_password("wrong-password", REF_HASH).expect("the hash is well-formed"));
    assert!(!verify_password("", REF_HASH).expect("the hash is well-formed"));
}

#[test]
fn our_hashes_cost_what_existing_hashes_did() {
    let ours = hash_password(REF_PASSWORD).expect("hashing succeeds");

    assert_eq!(
        cost_of(&ours),
        cost_of(REF_HASH),
        "changing the work factor silently re-prices every future login"
    );
}

#[test]
fn hashing_is_salted_and_verifiable() {
    let first = hash_password("hunter2-hunter2").expect("hashing succeeds");
    let second = hash_password("hunter2-hunter2").expect("hashing succeeds");

    assert_ne!(first, second, "each hash must carry its own random salt");
    assert!(verify_password("hunter2-hunter2", &first).expect("well-formed"));
    assert!(verify_password("hunter2-hunter2", &second).expect("well-formed"));
    assert!(!verify_password("hunter2-hunter3", &first).expect("well-formed"));
}

#[test]
fn an_over_long_password_is_refused_rather_than_truncated() {
    let too_long = "a".repeat(73);

    assert!(matches!(
        hash_password(&too_long),
        Err(crate::AuthError::PasswordTooLong)
    ));
    // The boundary itself (72 bytes) is still accepted.
    assert!(hash_password(&"a".repeat(72)).is_ok());
}

#[test]
fn a_malformed_stored_hash_is_an_error_not_a_silent_pass() {
    for corrupt in ["", "not-a-hash", "$2a$", "$2a$10$too-short"] {
        assert!(
            verify_password("anything", corrupt).is_err(),
            "{corrupt:?} must not be treated as a match"
        );
    }
}
