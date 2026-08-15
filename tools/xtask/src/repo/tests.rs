use std::path::Path;

use super::*;
use crate::fixture::Fixture;

#[test]
fn members_are_the_directories_that_carry_a_manifest() {
    let fixture = Fixture::new("repo-members");
    fixture
        .member("gw-beta", "")
        .member("gw-alpha", "")
        // A stray directory without a manifest is not a member.
        .write("crates/notacrate/README.md", "")
        .write("tools/xtask/Cargo.toml", "[package]\nname = \"xtask\"\n");

    let members = fixture.repo().members().expect("members");
    let names: Vec<&str> = members.iter().map(|m| m.dir_name.as_str()).collect();

    assert_eq!(
        names,
        ["gw-alpha", "gw-beta", "xtask"],
        "sorted, crates then tools"
    );
    assert_eq!(members[0].manifest, Path::new("crates/gw-alpha/Cargo.toml"));
}

#[test]
fn a_member_is_a_library_only_when_src_lib_rs_exists() {
    let fixture = Fixture::new("repo-is-lib");
    fixture
        .member("gw-lib", "")
        .write("crates/gw-bin/Cargo.toml", "[package]\nname = \"gw-bin\"\n")
        .write("crates/gw-bin/src/main.rs", "fn main() {}\n");

    let repo = fixture.repo();
    let members = repo.members().expect("members");
    assert!(!members[0].is_lib(&repo), "gw-bin has only main.rs");
    assert!(members[1].is_lib(&repo));
}

#[test]
fn walking_skips_build_output() {
    let fixture = Fixture::new("repo-walk");
    fixture
        .member("gw-x", "")
        .write("crates/gw-x/src/inner/deep.rs", "")
        .write("crates/gw-x/target/debug/generated.rs", "");

    let files = fixture
        .repo()
        .rust_files_under(Path::new("crates/gw-x"))
        .expect("walk");
    let shown: Vec<String> = files.iter().map(|p| display(p)).collect();

    assert_eq!(
        shown,
        ["crates/gw-x/src/inner/deep.rs", "crates/gw-x/src/lib.rs"]
    );
}

#[test]
fn walking_a_missing_directory_is_empty_not_an_error() {
    // Most members have no `tests/` directory; every gate walks for one.
    let fixture = Fixture::new("repo-missing");
    let files = fixture
        .repo()
        .rust_files_under(Path::new("crates/nope/tests"))
        .expect("walk");
    assert!(files.is_empty());
}

#[test]
fn sections_split_on_headers_and_drop_comments() {
    let sections = toml_sections(
        "\
name = \"root\"
# a comment
[package]
name = \"gw-x\"      # trailing comment

[[bin]]
path = \"src/main.rs\"
",
    );

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].header, "");
    assert_eq!(sections[0].lines[0].1, "name = \"root\"");
    assert_eq!(sections[1].header, "package");
    assert_eq!(sections[1].lines[0].1, "name = \"gw-x\"");
    // Array-of-table headers keep their inner name.
    assert_eq!(sections[2].header, "bin");
}

#[test]
fn section_lines_carry_their_line_numbers() {
    let sections = toml_sections("[package]\n\nname = \"x\"\n");
    assert_eq!(sections[1].lines, [(3, "name = \"x\"".to_owned())]);
}

#[test]
fn values_are_read_by_exact_key() {
    assert_eq!(toml_value("doctest = false", "doctest"), Some("false"));
    assert_eq!(toml_value("name = \"gw-x\"", "name"), Some("gw-x"));
    // A key that merely contains the name must not match: `doctest-extra` is
    // not `doctest`.
    assert_eq!(toml_value("doctest-extra = false", "doctest"), None);
    assert_eq!(toml_value("no equals here", "name"), None);
}
