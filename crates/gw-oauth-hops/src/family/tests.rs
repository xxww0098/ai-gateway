use super::Family;

/// Distinct families keep distinct keys so a pin map keyed by family cannot
/// collide two vendors onto one slot.
#[test]
fn family_keys_are_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for family in Family::ALL {
        let key = family.as_str();
        assert!(!key.is_empty());
        assert!(seen.insert(key), "duplicate family key {key}");
    }
    assert_eq!(seen.len(), Family::ALL.len());
}

/// A missing `src/<family>.rs` means the enum drifted from the tree.
#[test]
fn every_family_has_a_module_file() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for family in Family::ALL {
        let path = src.join(format!("{}.rs", family.as_str()));
        assert!(
            path.is_file(),
            "missing {} for Family::{}",
            path.display(),
            family.as_str()
        );
    }
}
