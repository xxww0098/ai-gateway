use super::{create_pkce, random_hex, random_token};

#[test]
fn pkce_challenge_is_s256_of_the_verifier() {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    let pair = create_pkce().expect("entropy");
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pair.verifier.as_bytes()));
    assert_eq!(pair.challenge, expected);
    assert_ne!(pair.verifier, pair.challenge);
}

#[test]
fn two_pkce_pairs_are_not_equal() {
    let a = create_pkce().expect("entropy");
    let b = create_pkce().expect("entropy");
    assert_ne!(a.verifier, b.verifier);
}

#[test]
fn random_hex_is_lowercase_hex_of_the_requested_length() {
    let nonce = random_hex(8).expect("entropy");
    assert_eq!(nonce.len(), 16);
    assert!(nonce.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn random_token_is_url_safe() {
    let token = random_token(32).expect("entropy");
    assert!(!token.contains('+'));
    assert!(!token.contains('/'));
    assert!(!token.contains('='));
}
