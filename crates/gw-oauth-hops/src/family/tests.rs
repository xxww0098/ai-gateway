use super::Family;

/// Distinct families keep distinct keys so a pin map keyed by family cannot
/// collide two vendors onto one slot.
#[test]
fn family_keys_are_unique() {
    let keys = [
        Family::Codex.as_str(),
        Family::Grok.as_str(),
        Family::Kiro.as_str(),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for key in keys {
        assert!(!key.is_empty());
        assert!(seen.insert(key), "duplicate family key {key}");
    }
}
