use super::*;

const PAIRED: &str = "\
[profile.dev]
opt-level = 0

[profile.dev.build-override]
opt-level = 3

[profile.dev.package.sqlx-macros]
opt-level = 3
";

const HALF: &str = "\
[profile.dev]
opt-level = 0

[profile.dev.package.sqlx-macros]
opt-level = 3
";

#[test]
fn profile_headers_split_into_profile_and_remainder() {
    let sections = profile_sections(PAIRED);
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].profile, "dev");
    assert_eq!(sections[0].rest, "");
    assert_eq!(sections[1].rest, "build-override");
    assert_eq!(sections[2].rest, "package.sqlx-macros");
}

#[test]
fn a_paired_profile_passes() {
    assert!(unpaired_overrides(PAIRED).is_empty());
}

#[test]
fn package_overrides_without_build_override_are_reported() {
    // Measured on this workspace: this half-configuration is SLOWER than doing
    // nothing at all (269s vs 255s).
    assert_eq!(unpaired_overrides(HALF), ["dev"]);
}

#[test]
fn each_profile_is_paired_independently() {
    let manifest = format!("{PAIRED}\n[profile.release.package.serde]\nopt-level = 3\n");
    assert_eq!(unpaired_overrides(&manifest), ["release"]);
}

#[test]
fn build_override_alone_is_allowed() {
    // The current root manifest: build-override with no per-package overrides
    // is a valid resting state, and the pairing rule only bites in one
    // direction (overrides require build-override, not the reverse).
    assert!(unpaired_overrides("[profile.dev.build-override]\nopt-level = 3\n").is_empty());
}

#[test]
fn the_wildcard_package_override_is_reported() {
    let manifest = "[profile.dev.package.\"*\"]\nopt-level = 3\n";
    assert_eq!(wildcard_overrides(manifest), ["profile.dev.package.\"*\""]);
}

#[test]
fn a_named_package_override_is_not_a_wildcard() {
    assert!(wildcard_overrides(PAIRED).is_empty());
    assert!(wildcard_overrides("[profile.dev.package.serde_derive]\nopt-level = 3\n").is_empty());
}

#[test]
fn the_real_root_manifest_passes_both_profile_gates() {
    let repo = crate::repo::Repo::discover().expect("workspace root");
    assert!(check_build_override_pair(&repo).expect("gate").is_empty());
    assert!(check_no_wildcard_opt(&repo).expect("gate").is_empty());
}
