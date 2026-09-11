use super::{Family, Hop};

#[test]
fn every_family_round_trips_through_as_str() {
    for family in Family::ALL {
        assert_eq!(Family::parse(family.as_str()), Some(family));
    }
}

#[test]
fn auth_url_keys_round_trip_except_glm_aliases() {
    for family in Family::ALL {
        assert_eq!(Family::from_auth_url_key(family.auth_url_key()), Some(family));
    }
}

#[test]
fn deleted_gemini_cli_oauth_is_not_a_family() {
    assert_eq!(Family::parse("gemini"), None);
    assert_eq!(Family::from_auth_url_key("gemini-cli-auth-url"), None);
    assert_eq!(Family::parse("xai"), None);
    assert_eq!(Family::from_auth_url_key("xai-auth-url"), None);
}

#[test]
fn glm_region_aliases_store_as_glm() {
    assert_eq!(Family::parse("zai").map(Family::as_str), Some("glm"));
    assert_eq!(Family::parse("bigmodel").map(Family::as_str), Some("glm"));
}

#[test]
fn hop_matches_vendor_native_wire() {
    assert_eq!(Family::Codex.hop(), Hop::Responses);
    assert_eq!(Family::Grok.hop(), Hop::Responses);
    assert_eq!(Family::Glm.hop(), Hop::Anthropic);
    assert_eq!(Family::Claude.hop(), Hop::Anthropic);
    assert_eq!(Family::Kiro.hop(), Hop::Completions);
    assert_eq!(Family::Cursor.hop(), Hop::Completions);
}
