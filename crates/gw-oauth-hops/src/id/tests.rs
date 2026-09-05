use super::{first_cache_id, sanitize_cache_id};

fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-')
}

/// Empty / whitespace never becomes a generated id. A timestamp fallback
/// would bust every cache on the next hop.
#[test]
fn blank_input_is_absent() {
    assert_eq!(sanitize_cache_id(""), None);
    assert_eq!(sanitize_cache_id("   \t"), None);
    assert_eq!(first_cache_id([None, Some(""), Some("  ")]), None);
}

/// Whatever punctuation arrives, the wire id stays inside the charset and
/// never grows past 64.
#[test]
fn wire_ids_stay_in_charset_and_cap() {
    let samples = [
        "hello world!",
        "会话/abc",
        &"x".repeat(80),
        "ok.thread_1:2",
        "\nfoo\n",
    ];
    for sample in samples {
        let Some(id) = sanitize_cache_id(sample) else {
            panic!("non-blank sample must sanitize: {sample:?}");
        };
        assert!(id.len() <= 64, "over cap: {id}");
        assert!(id.chars().all(allowed), "charset break: {id}");
        assert!(
            !id.chars().any(char::is_whitespace),
            "whitespace survived: {id}"
        );
    }
}

/// Later empty candidates do not wipe an earlier usable one.
#[test]
fn first_usable_candidate_wins() {
    let id = first_cache_id([Some("  "), Some("sess-a"), Some("sess-b")]).unwrap();
    assert!(id.starts_with("sess-"));
    assert_ne!(id, first_cache_id([Some("sess-b")]).unwrap());
}
