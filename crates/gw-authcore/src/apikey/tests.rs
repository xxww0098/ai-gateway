//! The SHA-256 vectors were produced by an independent implementation of the
//! same hash, so they pin cross-implementation identity rather than restating
//! this module.

use super::{API_KEY_PREFIX, KEY_PREFIX_LEN, api_key_prefix, hash_api_key, new_api_key};
use std::collections::HashSet;

#[test]
fn hash_matches_the_bytes_written_into_key_hash() {
    assert_eq!(
        hash_api_key("agw-0123456789abcdef"),
        "dc05fc4f6597e7de76f2f503997c2398a46f1d25ba93d5092f96995cf174e775"
    );
    assert_eq!(
        hash_api_key("agw-golden-fixture"),
        "34bb75cd4ef69ddd3c28fb94b4c20ebfb90924b46cb13d13ad51c16a53085cb9"
    );
    assert_eq!(
        hash_api_key(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hash_is_lowercase_hex_of_a_fixed_width() {
    let hash = hash_api_key("agw-whatever");

    assert_eq!(hash.len(), 64);
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "key_hash is looked up by exact string match; uppercase would miss"
    );
}

#[test]
fn prefix_takes_the_leading_window_and_short_input_whole() {
    let key = "agw-0123456789abcdef";

    assert_eq!(api_key_prefix(key).len(), KEY_PREFIX_LEN);
    assert!(key.starts_with(api_key_prefix(key)));
    assert_eq!(api_key_prefix("agw-"), "agw-");
    assert_eq!(api_key_prefix(""), "");
}

#[test]
fn prefix_never_splits_a_multibyte_character() {
    // Not a key we would ever mint, but a hand-typed one must not panic.
    let prefixed = api_key_prefix("日本語のキーです");

    assert!(prefixed.len() <= KEY_PREFIX_LEN);
    assert!("日本語のキーです".starts_with(prefixed));
}

#[test]
fn minted_keys_are_prefixed_hex_and_unique() {
    let mut seen = HashSet::new();
    for _ in 0..64 {
        let key = new_api_key().expect("OS entropy is available");

        assert!(key.starts_with(API_KEY_PREFIX));
        let body = &key[API_KEY_PREFIX.len()..];
        assert_eq!(body.len(), 64, "32 random bytes, hex-encoded");
        assert!(body.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(
            seen.insert(key),
            "a repeated key would collapse two accounts"
        );
    }
}

#[test]
fn minted_keys_survive_the_prefix_then_hash_round_trip() {
    let key = new_api_key().expect("OS entropy is available");

    assert_eq!(api_key_prefix(&key).len(), KEY_PREFIX_LEN);
    assert_eq!(hash_api_key(&key), hash_api_key(&key.clone()));
    assert_ne!(
        hash_api_key(&key),
        hash_api_key(api_key_prefix(&key)),
        "the stored prefix must not be enough to reconstruct the lookup hash"
    );
}
