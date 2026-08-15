use super::*;
use crate::fixture::Fixture;

#[test]
fn a_test_that_returns_when_the_environment_is_missing_is_reported() {
    let source = "\
#[test]
fn needs_postgres() {
    let url = match std::env::var(\"DATABASE_URL\") {
        Ok(url) => url,
        Err(_) => return,
    };
    let _ = url;
}
";
    let found = silent_skips(source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 3);
}

#[test]
fn failing_loudly_on_the_missing_variable_is_the_right_answer() {
    // Rule 2.9's first sanctioned option: tell the human how to fix it.
    let source = "\
#[test]
fn needs_postgres() {
    let url = std::env::var(\"DATABASE_URL\")
        .expect(\"set DATABASE_URL=postgres://... to run the ledger tests\");
    let _ = url;
}
";
    assert!(silent_skips(source).is_empty());
}

#[test]
fn an_ignored_test_is_the_other_right_answer() {
    let source = "\
#[test]
#[ignore = \"needs a local Postgres\"]
fn needs_postgres() {
    let url = std::env::var(\"DATABASE_URL\").unwrap();
    let _ = url;
}
";
    assert!(silent_skips(source).is_empty());
}

#[test]
fn a_return_far_from_the_env_read_is_not_a_skip() {
    let source = "\
#[test]
fn reads_config() {
    let home = std::env::var(\"HOME\").unwrap_or_default();
    let parsed = parse(&home);
    assert!(parsed.is_ok());
    let other = compute();
    if other.is_none() {
        return;
    }
}
";
    assert!(
        silent_skips(source).is_empty(),
        "{:?}",
        silent_skips(source)
    );
}

#[test]
fn only_test_carrying_files_are_scanned() {
    let fixture = Fixture::new("hygiene-scope");
    fixture
        .member(
            "gw-x",
            // Production code may absolutely bail when a variable is unset.
            "pub fn feature() {\n    if std::env::var(\"FLAG\").is_err() { return; }\n}\n",
        )
        .write(
            "crates/gw-x/tests/it.rs",
            "#[test]\nfn t() {\n    if std::env::var(\"DB\").is_err() { return; }\n}\n",
        );

    let findings = check_no_silent_test_skip(&fixture.repo()).expect("gate");
    let files: Vec<&str> = findings.iter().filter_map(|f| f.file.as_deref()).collect();
    assert_eq!(files, ["crates/gw-x/tests/it.rs:3"]);
}

#[test]
fn a_file_over_the_limit_is_reported() {
    let fixture = Fixture::new("hygiene-length");
    let long = "// line\n".repeat(MAX_FILE_LINES + 1);
    fixture.member("gw-x", &long);

    let findings = check_file_length(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0]
            .message
            .contains(&(MAX_FILE_LINES + 1).to_string())
    );
}

#[test]
fn a_file_at_the_limit_is_fine() {
    let fixture = Fixture::new("hygiene-limit");
    fixture.member("gw-x", &"// line\n".repeat(MAX_FILE_LINES));
    assert!(check_file_length(&fixture.repo()).expect("gate").is_empty());
}

#[test]
fn the_allowlist_is_a_ratchet_that_only_shrinks() {
    // Every entry must still be over the limit; one that is not has to go, or
    // the list quietly re-authorizes the next file to take that name.
    let repo = crate::repo::Repo::discover().expect("workspace root");
    let findings = check_file_length(&repo).expect("gate");
    let stale: Vec<&str> = findings
        .iter()
        .filter(|f| f.message.contains("no longer over the limit"))
        .filter_map(|f| f.file.as_deref())
        .collect();
    assert!(
        stale.is_empty(),
        "remove from LONG_FILE_ALLOWLIST: {stale:?}"
    );
}

#[test]
fn quoted_example_code_is_not_scanned() {
    // The gate reads its own test fixtures otherwise — and every doc example
    // that shows the wrong way to write a test.
    let source = "\
#[test]
fn documents_the_antipattern() {
    let bad = \"if std::env::var(\\\"DB\\\").is_err() { return; }\";
    assert!(bad.contains(\"return\"));
}
";
    assert!(
        silent_skips(source).is_empty(),
        "{:?}",
        silent_skips(source)
    );
}

#[test]
fn the_hygiene_gates_run_against_the_real_tree() {
    // A smoke test for the filesystem walking, not for cleanliness: whether the
    // tree currently passes is `cargo xtask ci`'s answer to give, and these
    // gates cover files other workers own.
    let repo = crate::repo::Repo::discover().expect("workspace root");
    check_file_length(&repo).expect("file_length runs");
    check_no_silent_test_skip(&repo).expect("no_silent_test_skip runs");
}
