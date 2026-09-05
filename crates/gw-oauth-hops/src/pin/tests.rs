use super::PrefixPins;

/// The first blob for a conversation is what later hops must keep sending.
/// A longer snapshot of the same prefix is extra, not a rewrite of the pin.
#[test]
fn first_blob_sticks_and_growth_is_extra() {
    let mut pins = PrefixPins::new();
    let first = pins.pin("conv-1", "you are a helper", &[]);
    assert_eq!(first.pinned, "you are a helper");
    assert!(first.extra.is_empty());

    let grown = pins.pin(
        "conv-1",
        "you are a helper\n\nthis snapshot supersedes",
        &[],
    );
    assert_eq!(grown.pinned, "you are a helper");
    assert!(!grown.extra.is_empty());
    assert_ne!(grown.extra, grown.pinned);
}

/// Fallback ids (`dsh-*`) are not stored. Storing them would let tenant B
/// inherit tenant A's prefix when both omitted a session id.
#[test]
fn fallback_ids_are_not_stored() {
    let mut pins = PrefixPins::new();
    let skip = ["dsh-grok"];
    let a = pins.pin("dsh-grok", "tenant-a system", &skip);
    let b = pins.pin("dsh-grok", "tenant-b system", &skip);
    assert_eq!(a.pinned, "tenant-a system");
    assert_eq!(b.pinned, "tenant-b system");
    assert!(a.extra.is_empty() && b.extra.is_empty());
}

/// Two conversations keep independent prefixes. A replacement that is not a
/// prefix of the original comes back as extra, not as a new pin.
#[test]
fn conversations_do_not_share_a_pin() {
    let mut pins = PrefixPins::new();
    let _ = pins.pin("a", "alpha", &[]);
    let _ = pins.pin("b", "beta", &[]);
    let again_a = pins.pin("a", "zeta", &[]);
    assert_eq!(again_a.pinned, "alpha");
    assert_eq!(again_a.extra, "zeta");
}
