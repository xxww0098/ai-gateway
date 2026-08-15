//! Root-manifest `[profile.*]` gates.
//!
//! * rule 3.3 — per-package `opt-level` overrides and `build-override` are a
//!   PAIR. Measured on this workspace: overrides without `build-override` was
//!   SLOWER than changing nothing (269s vs 255s); the pair was 75.8s. Deleting
//!   half of it is the kind of edit that looks like a cleanup and costs three
//!   minutes per build.
//! * rule 3.2 — `[profile.*.package."*"]` is banned outright (antipattern #1):
//!   the wildcard silently opts every dependency into a slower profile.

use std::path::Path;

use anyhow::Result;

use crate::gates::Finding;
use crate::repo::{Repo, toml_sections};

/// A parsed `[profile.<name>....]` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileSection {
    pub(crate) profile: String,
    /// The part after `profile.<name>.`, e.g. `package.foo` or
    /// `build-override`. Empty for the profile table itself.
    pub(crate) rest: String,
}

/// Extract the profile sections of a manifest.
pub(crate) fn profile_sections(manifest: &str) -> Vec<ProfileSection> {
    toml_sections(manifest)
        .into_iter()
        .filter_map(|section| {
            let rest = section.header.strip_prefix("profile.")?;
            let (profile, rest) = match rest.split_once('.') {
                Some((profile, rest)) => (profile, rest),
                None => (rest, ""),
            };
            Some(ProfileSection {
                profile: profile.to_owned(),
                rest: rest.to_owned(),
            })
        })
        .collect()
}

/// Rule 3.3: a profile with per-package overrides must also carry
/// `build-override`.
pub(crate) fn unpaired_overrides(manifest: &str) -> Vec<String> {
    let sections = profile_sections(manifest);
    let mut unpaired = Vec::new();

    for section in &sections {
        if !section.rest.starts_with("package") {
            continue;
        }
        let paired = sections
            .iter()
            .any(|other| other.profile == section.profile && other.rest == "build-override");
        if !paired && !unpaired.contains(&section.profile) {
            unpaired.push(section.profile.clone());
        }
    }

    unpaired
}

/// Rule 3.2: `package."*"` in any profile.
pub(crate) fn wildcard_overrides(manifest: &str) -> Vec<String> {
    profile_sections(manifest)
        .into_iter()
        .filter(|section| {
            let package = section.rest.strip_prefix("package");
            package.is_some_and(|rest| {
                let key = rest.trim_start_matches('.').trim_matches('"');
                key == "*"
            })
        })
        .map(|section| format!("profile.{}.{}", section.profile, section.rest))
        .collect()
}

/// Rule 3.3.
pub(crate) fn check_build_override_pair(repo: &Repo) -> Result<Vec<Finding>> {
    let manifest = repo.read(Path::new("Cargo.toml"))?;
    Ok(unpaired_overrides(&manifest)
        .into_iter()
        .map(|profile| Finding {
            file: Some("Cargo.toml".to_owned()),
            message: format!(
                "[profile.{profile}.package.*] exists without [profile.{profile}.build-override] — half of the pair is slower than neither half (269s vs 255s measured; both is 75.8s)"
            ),
        })
        .collect())
}

/// Rule 3.2.
pub(crate) fn check_no_wildcard_opt(repo: &Repo) -> Result<Vec<Finding>> {
    let manifest = repo.read(Path::new("Cargo.toml"))?;
    Ok(wildcard_overrides(&manifest)
        .into_iter()
        .map(|header| Finding {
            file: Some("Cargo.toml".to_owned()),
            message: format!(
                "[{header}] opts every dependency into the override (antipattern #1) — name the packages that actually need it"
            ),
        })
        .collect())
}

#[cfg(test)]
mod tests;
