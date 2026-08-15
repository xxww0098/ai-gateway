use super::*;
use crate::fixture::Fixture;

#[test]
fn target_layout_paths_are_not_dependency_paths() {
    // gw-server legitimately sets [lib] path / [[bin]] path. Flagging those
    // would make the gate cry wolf on the one crate that needs them.
    let manifest = "\
[package]
name = \"gw-server\"

[lib]
path = \"src/lib.rs\"

[[bin]]
path = \"src/main.rs\"

[dependencies]
gw-config.workspace = true
";
    assert!(raw_path_deps(manifest).is_empty());
}

#[test]
fn hand_written_dependency_paths_are_reported() {
    let manifest = "\
[dependencies]
gw-config = { path = \"../gw-config\" }

[dev-dependencies]
gw-testkit = { version = \"0.0.0\", path = \"../gw-testkit\" }

[build-dependencies]
gen = { path = \"../gen\" }
";
    let found = raw_path_deps(manifest);
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(found[0].0, 2, "line numbers point at the offending line");
}

#[test]
fn a_key_that_merely_ends_in_path_is_not_a_path_dependency() {
    let manifest = "[dependencies]\nthing = { search-path = \"x\" }\nother = { fpath = \"y\" }\n";
    assert!(
        raw_path_deps(manifest).is_empty(),
        "{:?}",
        raw_path_deps(manifest)
    );
}

#[test]
fn target_specific_dependency_tables_are_checked_too() {
    let manifest = "[target.'cfg(unix)'.dependencies]\nnix-helper = { path = \"../helper\" }\n";
    assert_eq!(raw_path_deps(manifest).len(), 1);
}

#[test]
fn doctest_off_is_read_from_the_lib_section_only() {
    assert!(declares_doctest_off("[lib]\ndoctest = false\n"));
    assert!(!declares_doctest_off("[lib]\ndoctest = true\n"));
    assert!(!declares_doctest_off("[package]\ndoctest = false\n"));
    assert!(!declares_doctest_off("[package]\nname = \"x\"\n"));
}

#[test]
fn the_package_name_comes_from_the_package_section() {
    assert_eq!(
        package_name("[package]\nname = \"gw-config\"\n").as_deref(),
        Some("gw-config")
    );
    // A dependency called `name` must not be mistaken for the package name.
    assert_eq!(
        package_name("[dependencies]\nname = \"nope\"\n\n[package]\nname = \"real\"\n").as_deref(),
        Some("real")
    );
}

#[test]
fn a_bin_only_member_is_exempt_from_the_doctest_rule() {
    // tools/xtask has no library target, so there are no doctests to disable.
    let fixture = Fixture::new("manifests-bin-only");
    fixture
        .write("tools/xtask/Cargo.toml", "[package]\nname = \"xtask\"\n")
        .write("tools/xtask/src/main.rs", "fn main() {}\n");

    assert!(
        check_internal_doctest_off(&fixture.repo())
            .expect("gate")
            .is_empty()
    );
}

#[test]
fn a_library_without_doctest_false_is_reported() {
    let fixture = Fixture::new("manifests-doctest");
    fixture
        .write("crates/gw-x/Cargo.toml", "[package]\nname = \"gw-x\"\n")
        .write("crates/gw-x/src/lib.rs", "");

    let findings = check_internal_doctest_off(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1, "{findings:?}");
}

#[test]
fn a_crate_whose_directory_disagrees_with_its_name_is_reported() {
    let fixture = Fixture::new("manifests-dir-name");
    fixture
        .write(
            "crates/gw-x/Cargo.toml",
            "[package]\nname = \"gateway-x\"\n[lib]\ndoctest = false\n",
        )
        .write("crates/gw-x/src/lib.rs", "");

    let findings = check_dir_name_matches_crate(&fixture.repo()).expect("gate");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("gateway-x"));
}

#[test]
fn the_real_workspace_passes_the_manifest_gates() {
    let repo = crate::repo::Repo::discover().expect("workspace root");
    assert!(check_no_raw_path_deps(&repo).expect("gate").is_empty());
    assert!(check_internal_doctest_off(&repo).expect("gate").is_empty());
    assert!(
        check_dir_name_matches_crate(&repo)
            .expect("gate")
            .is_empty()
    );
}
