use std::path::Path;

use super::*;
use crate::fixture::Fixture;

fn names(source: &str) -> Vec<String> {
    parse_mod_decls(source)
        .into_iter()
        .map(|decl| decl.name)
        .collect()
}

#[test]
fn every_visibility_of_an_external_module_is_a_declaration() {
    let decls = names(
        "\
mod plain;
pub mod exported;
pub(crate) mod internal;
pub(super) mod upward;
",
    );
    assert_eq!(decls, ["plain", "exported", "internal", "upward"]);
}

#[test]
fn inline_modules_declare_no_file() {
    // `mod tests { .. }` is what rule 2.2 forbids; it also points at nothing on
    // disk, so treating it as a declaration would invent a phantom file.
    assert!(names("mod tests {\n    #[test]\n    fn t() {}\n}\n").is_empty());
}

#[test]
fn commented_out_declarations_are_not_declarations() {
    assert!(names("// mod removed;\n/// mod documented;\n//! mod inner;\n").is_empty());
    assert_eq!(names("mod live; // mod dead;\n"), ["live"]);
}

#[test]
fn the_path_attribute_is_captured_through_intervening_attributes() {
    // gw-provider writes exactly this shape.
    let decls = parse_mod_decls("#[cfg(test)]\n#[path = \"codex_tests.rs\"]\nmod tests;\n");
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].path_attr.as_deref(), Some("codex_tests.rs"));
}

#[test]
fn a_path_attribute_does_not_leak_onto_a_later_module() {
    let decls = parse_mod_decls("#[path = \"a.rs\"]\nmod first;\nmod second;\n");
    assert_eq!(decls[0].path_attr.as_deref(), Some("a.rs"));
    assert_eq!(
        decls[1].path_attr, None,
        "the second module would resolve to the first one's file"
    );
}

#[test]
fn a_statement_between_attribute_and_module_clears_the_attribute() {
    let decls = parse_mod_decls("#[path = \"a.rs\"]\nuse std::io;\nmod second;\n");
    assert_eq!(decls[0].path_attr, None);
}

#[test]
fn mod_rs_files_own_their_own_directory() {
    let decl = ModDecl {
        name: "tests".to_owned(),
        path_attr: None,
        line: 1,
    };

    // lib.rs / main.rs / mod.rs resolve siblings...
    assert_eq!(
        candidate_paths(&decl, Path::new("crates/gw-x/src/lib.rs"), true),
        [
            Path::new("crates/gw-x/src/tests.rs"),
            Path::new("crates/gw-x/src/tests/mod.rs")
        ]
    );
    // ...while any other file resolves into a directory named after itself.
    assert_eq!(
        candidate_paths(&decl, Path::new("crates/gw-x/src/health.rs"), false),
        [
            Path::new("crates/gw-x/src/health/tests.rs"),
            Path::new("crates/gw-x/src/health/tests/mod.rs")
        ]
    );
}

#[test]
fn an_integration_test_root_resolves_siblings_like_main_rs() {
    // Every `tests/*.rs` is its own crate root, so `mod common;` there means
    // `tests/common/mod.rs` — not `tests/<file>/common.rs` (rules 2.3 / 2.8).
    let decl = ModDecl {
        name: "common".to_owned(),
        path_attr: None,
        line: 1,
    };
    assert_eq!(
        candidate_paths(&decl, Path::new("crates/gw-x/tests/billing.rs"), true),
        [
            Path::new("crates/gw-x/tests/common.rs"),
            Path::new("crates/gw-x/tests/common/mod.rs")
        ]
    );
}

#[test]
fn a_path_attribute_resolves_against_the_declaring_files_directory() {
    // rustc's rule for non-inline modules — NOT the `health/` subdirectory.
    let decl = ModDecl {
        name: "tests".to_owned(),
        path_attr: Some("health_tests.rs".to_owned()),
        line: 1,
    };
    assert_eq!(
        candidate_paths(&decl, Path::new("crates/gw-x/src/health.rs"), false),
        [Path::new("crates/gw-x/src/health_tests.rs")]
    );
}

#[test]
fn a_reachable_tree_has_no_orphans() {
    let fixture = Fixture::new("modules-clean");
    fixture
        .member("gw-x", "mod health;\n#[cfg(test)]\nmod tests;\n")
        .write("crates/gw-x/src/health.rs", "#[cfg(test)]\nmod tests;\n")
        .write("crates/gw-x/src/health/tests.rs", "#[test]\nfn t() {}\n")
        .write("crates/gw-x/src/tests.rs", "");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn an_undeclared_file_is_reported_with_the_tests_it_costs() {
    let fixture = Fixture::new("modules-orphan");
    fixture
        .member("gw-x", "mod health;\n")
        .write("crates/gw-x/src/health.rs", "")
        // Nothing declares this: it compiles never and reviews clean.
        .write(
            "crates/gw-x/src/health/tests.rs",
            "#[test]\nfn a() {}\n#[tokio::test]\nasync fn b() {}\n",
        );

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0].file.as_deref(),
        Some("crates/gw-x/src/health/tests.rs")
    );
    assert!(findings[0].message.contains('2'), "{}", findings[0].message);
}

#[test]
fn a_path_attribute_keeps_its_target_reachable() {
    let fixture = Fixture::new("modules-path-attr");
    fixture
        .member("gw-x", "mod codex;\n")
        .write(
            "crates/gw-x/src/codex.rs",
            "#[cfg(test)]\n#[path = \"codex_tests.rs\"]\nmod tests;\n",
        )
        .write("crates/gw-x/src/codex_tests.rs", "#[test]\nfn t() {}\n");

    assert!(
        check_no_orphan_modules(&fixture.repo())
            .expect("gate")
            .is_empty()
    );
}

#[test]
fn integration_test_roots_keep_their_shared_helper_alive() {
    let fixture = Fixture::new("modules-tests-dir");
    fixture
        .member("gw-x", "")
        .write("crates/gw-x/tests/billing.rs", "mod common;\n")
        .write("crates/gw-x/tests/common/mod.rs", "pub fn helper() {}\n")
        // rules 2.3 / 2.8: nothing declares this one.
        .write("crates/gw-x/tests/orphan/mod.rs", "#[test]\nfn t() {}\n");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    let files: Vec<&str> = findings.iter().filter_map(|f| f.file.as_deref()).collect();
    assert_eq!(files, ["crates/gw-x/tests/orphan/mod.rs"]);
}

#[test]
fn an_explicit_target_path_is_read_out_of_the_manifest() {
    // gw-panel's shape: rule 2.8's single integration binary, declared because
    // the file it points at is not `tests/<name>.rs`.
    let targets = parse_manifest_targets(
        "\
[package]
name = \"gw-panel\"

[lib]
doctest = false

[[test]]
name = \"panel\"
path = \"tests/panel/main.rs\"

[dependencies]
gw-model = { path = \"../gw-model\" }
",
    );
    assert_eq!(targets.paths, ["tests/panel/main.rs"]);
    assert!(
        targets.autotests && targets.autobins && targets.autolib,
        "an explicit target does not switch discovery off in edition 2024"
    );
}

#[test]
fn the_path_of_a_dependency_is_not_a_crate_root() {
    // The one way a line scanner could invent a root: `path` means something
    // else entirely outside a target section.
    let targets = parse_manifest_targets("[dependencies]\ngw-model = { path = \"../gw-model\" }\n");
    assert!(targets.paths.is_empty());
}

#[test]
fn discovery_switches_are_honoured() {
    let targets = parse_manifest_targets("[package]\nautotests = false\nautobins = false\n");
    assert!(!targets.autotests);
    assert!(!targets.autobins);
    assert!(targets.autolib, "only the keys present are switched off");
}

#[test]
fn an_explicitly_declared_test_target_is_a_crate_root() {
    let fixture = Fixture::new("modules-explicit-test-target");
    fixture
        .member("gw-x", "")
        .write(
            "crates/gw-x/Cargo.toml",
            "[package]\nname = \"gw-x\"\n\n[[test]]\nname = \"panel\"\npath = \"tests/panel/main.rs\"\n",
        )
        .write("crates/gw-x/tests/panel/main.rs", "mod common;\nmod billing;\n")
        .write("crates/gw-x/tests/panel/common/mod.rs", "pub fn helper() {}\n")
        .write("crates/gw-x/tests/panel/billing.rs", "#[test]\nfn t() {}\n");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    assert!(
        findings.is_empty(),
        "the manifest names this crate root, so nothing under it is orphaned: {findings:?}"
    );
}

#[test]
fn an_undeclared_sibling_of_an_explicit_root_is_still_reported() {
    // The negative control for the test above: widening the roots must not
    // widen reachability. A file the root does not declare is still dead.
    let fixture = Fixture::new("modules-explicit-target-orphan");
    fixture
        .member("gw-x", "")
        .write(
            "crates/gw-x/Cargo.toml",
            "[package]\nname = \"gw-x\"\n\n[[test]]\nname = \"panel\"\npath = \"tests/panel/main.rs\"\n",
        )
        .write("crates/gw-x/tests/panel/main.rs", "mod billing;\n")
        .write("crates/gw-x/tests/panel/billing.rs", "")
        .write("crates/gw-x/tests/panel/refunds.rs", "#[test]\nfn t() {}\n");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    let files: Vec<&str> = findings.iter().filter_map(|f| f.file.as_deref()).collect();
    assert_eq!(files, ["crates/gw-x/tests/panel/refunds.rs"]);
}

#[test]
fn a_tests_subdirectory_main_rs_is_discovered_without_a_manifest_entry() {
    // Cargo discovers `tests/*/main.rs` on its own; the manifest entry above is
    // belt and braces, not the thing that makes the layout legal.
    let fixture = Fixture::new("modules-tests-main-rs");
    fixture
        .member("gw-x", "")
        .write("crates/gw-x/tests/panel/main.rs", "mod common;\n")
        .write(
            "crates/gw-x/tests/panel/common/mod.rs",
            "pub fn helper() {}\n",
        );

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn autotests_false_takes_the_discovered_roots_away() {
    // Switched off, the file really is never compiled — reporting it is the
    // gate doing its job, not a false positive.
    let fixture = Fixture::new("modules-autotests-off");
    fixture
        .member("gw-x", "")
        .write(
            "crates/gw-x/Cargo.toml",
            "[package]\nname = \"gw-x\"\nautotests = false\n",
        )
        .write("crates/gw-x/tests/billing.rs", "#[test]\nfn t() {}\n");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    let files: Vec<&str> = findings.iter().filter_map(|f| f.file.as_deref()).collect();
    assert_eq!(files, ["crates/gw-x/tests/billing.rs"]);
}

#[test]
fn a_binary_under_src_bin_is_a_crate_root() {
    let fixture = Fixture::new("modules-src-bin");
    fixture
        .member("gw-x", "")
        .write(
            "crates/gw-x/src/bin/seed.rs",
            "mod support;\nfn main() {}\n",
        )
        .write("crates/gw-x/src/bin/support.rs", "")
        .write("crates/gw-x/src/bin/migrate/main.rs", "fn main() {}\n");

    let findings = check_no_orphan_modules(&fixture.repo()).expect("gate");
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn the_graph_records_who_declared_what() {
    let fixture = Fixture::new("modules-graph");
    fixture
        .member("gw-x", "mod health;\n")
        .write("crates/gw-x/src/health.rs", "#[cfg(test)]\nmod tests;\n")
        .write("crates/gw-x/src/health/tests.rs", "");

    let repo = fixture.repo();
    let member = repo.members().expect("members").remove(0);
    let graph = graph_of(&repo, &member).expect("graph");

    // Ownership inheritance rides on this edge.
    assert_eq!(
        graph
            .parents
            .get(Path::new("crates/gw-x/src/health/tests.rs")),
        Some(&Path::new("crates/gw-x/src/health.rs").to_path_buf())
    );
    assert_eq!(
        graph.ancestor(Path::new("crates/gw-x/src/health/tests.rs"), |path| path
            .ends_with("lib.rs")),
        Some(Path::new("crates/gw-x/src/lib.rs").to_path_buf())
    );
}

#[test]
fn the_gate_walks_the_real_workspace() {
    // A smoke test for the walking and parsing against a real tree — every
    // member crate, `#[path]` attributes, `tests/` roots and all.
    //
    // Deliberately NOT an assertion that the tree is clean: nine workers are
    // editing it, and a half-written module is a finding for `cargo xtask ci`
    // to report, not a reason for xtask's own suite to go red.
    let repo = crate::repo::Repo::discover().expect("workspace root");
    let members = repo.members().expect("members");
    assert!(members.len() > 1, "expected a populated workspace");
    check_no_orphan_modules(&repo).expect("gate runs");

    // Whatever a manifest declares explicitly must survive into the roots.
    // Stated against the real tree rather than a fixed crate name so it keeps
    // holding as members come and go.
    for member in &members {
        let manifest = repo.read(&member.manifest).expect("read manifest");
        let roots = crate_roots(&repo, member).expect("roots");
        for declared in parse_manifest_targets(&manifest).paths {
            let path = member.dir.join(&declared);
            if repo.absolute(&path).is_file() {
                assert!(
                    roots.contains(&path),
                    "{} declares {declared}, which is a crate root by definition",
                    display(&member.manifest)
                );
            }
        }
    }

    // What must hold unconditionally: this crate's own files are reachable.
    let xtask = members
        .iter()
        .find(|member| member.dir_name == "xtask")
        .expect("xtask is a member");
    let graph = graph_of(&repo, xtask).expect("graph");
    let reachable = graph.reachable();
    for file in repo
        .rust_files_under(&xtask.dir.join("src"))
        .expect("walk src")
    {
        assert!(reachable.contains(&file), "{} is orphaned", file.display());
    }
}
