use super::{is_known_bad, mark_failed, mark_succeeded, reset};

#[test]
fn empty_token_is_never_known_bad() {
    reset();
    assert!(!is_known_bad("", 0));
    assert!(!is_known_bad("   ", 1));
    mark_failed("", 0);
    assert!(!is_known_bad("", 0));
}

#[test]
fn failure_is_temporary_and_success_clears_it() {
    reset();
    mark_failed("rt", 0);
    assert!(is_known_bad("rt", 0));
    assert!(is_known_bad(" rt ", 1));
    assert!(!is_known_bad("rt", i64::MAX));
    mark_failed("rt", 50);
    assert!(is_known_bad("rt", 50));
    mark_succeeded("rt");
    assert!(!is_known_bad("rt", 50));
    reset();
    assert!(!is_known_bad("rt", 0));
}
