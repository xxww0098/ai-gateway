use super::*;
use crate::fixture::Fixture;

const TABLE: &str = "\
# heading

## 3. 文件所有权（不可越界）

### 第一波

| worker | 独占目录 / 文件 |
| --- | --- |
| `research` | `rust/docs/**` |
| `platform` | `crates/gw-config/**`、`crates/gw-server/**`、`tools/xtask/**` |
| `provider-openai` | `crates/gw-provider/src/{common,openai}.rs` |

**协调者独占**（要改就 `ask`，别自己动手）：
`rust/Cargo.toml`、`crates/*/Cargo.toml`、
`crates/gw-provider/src/{lib,types}.rs`。

## 4. 参考实现对照表

| `platform` | `should/not/be/parsed/**` |
";

#[test]
fn table_rows_and_the_coordinator_block_are_both_claims() {
    let claims = parse_claims(TABLE);

    let platform: Vec<&str> = claims
        .iter()
        .filter(|c| c.owner == "platform")
        .map(|c| c.pattern.as_str())
        .collect();
    assert_eq!(
        platform,
        [
            "crates/gw-config/**",
            "crates/gw-server/**",
            "tools/xtask/**"
        ],
        "the 、-separated cell must split into three claims"
    );

    let coordinator: Vec<&str> = claims
        .iter()
        .filter(|c| c.is_coordinator())
        .map(|c| c.pattern.as_str())
        .collect();
    assert_eq!(
        coordinator,
        [
            "rust/Cargo.toml",
            "crates/*/Cargo.toml",
            "crates/gw-provider/src/{lib,types}.rs"
        ],
        "the prose block after the tables counts too"
    );
}

#[test]
fn parsing_stops_at_the_next_section() {
    // §4 is a different table entirely; claiming its cells would hand `platform`
    // paths it does not own.
    let claims = parse_claims(TABLE);
    assert!(!claims.iter().any(|c| c.pattern.contains("should/not")));
}

#[test]
fn a_worker_name_is_not_mistaken_for_a_path() {
    let claims = parse_claims(TABLE);
    assert!(!claims.iter().any(|c| c.pattern == "research"));
}

#[test]
fn patterns_are_normalized_to_the_workspace_root() {
    // The table writes some paths from the git root and some from `rust/`.
    assert_eq!(normalize("rust/docs/**"), "docs/**");
    assert_eq!(normalize("crates/gw-config/**"), "crates/gw-config/**");
}

#[test]
fn double_star_claims_the_whole_subtree() {
    assert!(glob_match(
        "crates/gw-config/**",
        "crates/gw-config/src/lib.rs"
    ));
    assert!(glob_match(
        "crates/gw-config/**",
        "crates/gw-config/Cargo.toml"
    ));
    assert!(!glob_match(
        "crates/gw-config/**",
        "crates/gw-server/src/lib.rs"
    ));
}

#[test]
fn single_star_stays_inside_one_segment() {
    assert!(glob_match(
        "crates/*/Cargo.toml",
        "crates/gw-config/Cargo.toml"
    ));
    assert!(
        !glob_match("crates/*/Cargo.toml", "crates/gw-config/src/Cargo.toml"),
        "* must not swallow a path separator"
    );
}

#[test]
fn brace_alternatives_expand() {
    let pattern = "crates/gw-provider/src/{common,openai,codex}.rs";
    assert!(glob_match(pattern, "crates/gw-provider/src/openai.rs"));
    assert!(glob_match(pattern, "crates/gw-provider/src/codex.rs"));
    assert!(!glob_match(pattern, "crates/gw-provider/src/claude.rs"));
    assert!(
        !glob_match(pattern, "crates/gw-provider/src/openai_tests.rs"),
        "an exact-file claim must not spill onto its neighbours"
    );
}

#[test]
fn nested_directory_patterns_match_at_any_depth() {
    let pattern = "crates/gw-panel/src/{identity,commerce}/**";
    assert!(glob_match(pattern, "crates/gw-panel/src/identity/mod.rs"));
    assert!(glob_match(
        pattern,
        "crates/gw-panel/src/commerce/refund/api.rs"
    ));
    assert!(!glob_match(pattern, "crates/gw-panel/src/ops/mod.rs"));
}

// ---------------------------------------------------------------------------
// The gate itself
// ---------------------------------------------------------------------------

/// A fixture whose CONTRACT.md carries `table` and whose tree is `files`.
fn fixture_with(name: &str, table: &str, build: impl Fn(&Fixture)) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write("CONTRACT.md", table);
    build(&fixture);
    fixture
}

const MINIMAL_TABLE: &str = "\
## 3. 文件所有权

| worker | 独占目录 / 文件 |
| --- | --- |
| `alpha` | `crates/gw-x/src/health.rs` |

**协调者独占**：`Cargo.toml`、`CONTRACT.md`、`crates/*/Cargo.toml`、`crates/gw-x/src/lib.rs`。

## 4. next
";

#[test]
fn a_fully_claimed_tree_is_clean() {
    let fixture = fixture_with("ownership-clean", MINIMAL_TABLE, |f| {
        f.member("gw-x", "mod health;\n")
            .write("crates/gw-x/src/health.rs", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn a_file_nobody_claims_is_reported() {
    let fixture = fixture_with("ownership-gap", MINIMAL_TABLE, |f| {
        f.member("gw-x", "mod health;\n")
            .write("crates/gw-x/src/health.rs", "")
            // Declared by lib.rs, which the coordinator owns — but this new
            // top-level module belongs to nobody in the table.
            .write("crates/gw-x/src/rogue.rs", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    let files: Vec<&str> = findings.iter().filter_map(|f| f.file.as_deref()).collect();
    assert!(files.contains(&"crates/gw-x/src/rogue.rs"), "{findings:#?}");
}

#[test]
fn ownership_is_inherited_through_the_module_graph() {
    // `health/tests.rs` is not in the table, but only `health.rs` can reach it,
    // so listing entry points is enough — the table does not have to enumerate
    // every test file a worker adds.
    let fixture = fixture_with("ownership-inherit", MINIMAL_TABLE, |f| {
        f.member("gw-x", "mod health;\n")
            .write("crates/gw-x/src/health.rs", "#[cfg(test)]\nmod tests;\n")
            .write("crates/gw-x/src/health/tests.rs", "#[test]\nfn t() {}\n");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn a_pattern_matching_nothing_is_reported_as_stale() {
    let table = MINIMAL_TABLE.replace(
        "| `alpha` | `crates/gw-x/src/health.rs` |",
        "| `alpha` | `crates/gw-x/src/health.rs`、`crates/gw-deleted/**` |",
    );
    let fixture = fixture_with("ownership-stale", &table, |f| {
        f.member("gw-x", "mod health;\n")
            .write("crates/gw-x/src/health.rs", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert!(
        findings[0].message.contains("crates/gw-deleted/**"),
        "{}",
        findings[0].message
    );
}

#[test]
fn two_workers_claiming_one_file_is_reported() {
    let table = MINIMAL_TABLE.replace(
        "| `alpha` | `crates/gw-x/src/health.rs` |",
        "| `alpha` | `crates/gw-x/src/health.rs` |\n| `beta` | `crates/gw-x/src/**` |",
    );
    let fixture = fixture_with("ownership-conflict", &table, |f| {
        f.member("gw-x", "mod health;\n")
            .write("crates/gw-x/src/health.rs", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    let conflict = findings
        .iter()
        .find(|f| f.file.as_deref() == Some("crates/gw-x/src/health.rs"))
        .expect("conflict reported");
    assert!(conflict.message.contains("alpha"), "{}", conflict.message);
    assert!(conflict.message.contains("beta"), "{}", conflict.message);
}

#[test]
fn a_coordinator_claim_overrides_a_worker_claim_instead_of_conflicting() {
    // `crates/*/Cargo.toml` (coordinator) necessarily overlaps every worker's
    // `crates/<x>/**`. That is the table's design, not a violation.
    let table = MINIMAL_TABLE.replace(
        "| `alpha` | `crates/gw-x/src/health.rs` |",
        "| `alpha` | `crates/gw-x/**` |",
    );
    let fixture = fixture_with("ownership-override", &table, |f| {
        f.member("gw-x", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn a_table_that_parses_to_nothing_fails_loudly() {
    // Otherwise renaming the section turns this gate into a no-op that reports
    // success forever.
    let fixture = fixture_with("ownership-empty", "# no section 3 here\n", |f| {
        f.member("gw-x", "");
    });

    let findings = check_file_ownership(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].file.as_deref(), Some("CONTRACT.md"));
}
