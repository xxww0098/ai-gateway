//! Per-member `Cargo.toml` gates.
//!
//! * rule 1.12 — members never hand-write `path = "../x"`; they say
//!   `gw-config.workspace = true` so the root manifest stays the single place a
//!   dependency edge is declared.
//! * rule 2.10 — internal crates set `[lib] doctest = false`; nobody outside
//!   this repo reads their doc examples, and compiling them costs a full extra
//!   test binary per crate.
//! * rule 1.3 — directory name = crate name, so a path in the ownership table
//!   and a `-p` flag name the same thing. `tools/` is the documented exception.

use std::path::Path;

use anyhow::Result;

use crate::gates::Finding;
use crate::repo::{Repo, display, toml_sections, toml_value};

/// Sections whose `path = ` keys are dependency edges rather than target
/// layout (`[lib] path = "src/lib.rs"` is legitimate and must not be flagged).
fn is_dependency_section(header: &str) -> bool {
    let tail = header.rsplit('.').next().unwrap_or(header);
    matches!(
        tail,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    )
}

/// Hand-written path dependencies in one manifest, as `(line, text)`.
pub(crate) fn raw_path_deps(manifest: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for section in toml_sections(manifest) {
        if !is_dependency_section(&section.header) {
            continue;
        }
        for (line, text) in section.lines {
            if text.contains("path")
                && text.contains('=')
                && text.contains('"')
                && has_path_key(&text)
            {
                out.push((line, text));
            }
        }
    }
    out
}

/// `path = "..."` as a bare key or inside an inline table.
fn has_path_key(line: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find("path") {
        let before = rest[..at].chars().next_back();
        let after = rest[at + 4..].trim_start();
        let boundary = before.is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-');
        if boundary && after.starts_with('=') {
            return true;
        }
        rest = &rest[at + 4..];
    }
    false
}

/// Whether a manifest declares `[lib] doctest = false`.
pub(crate) fn declares_doctest_off(manifest: &str) -> bool {
    toml_sections(manifest)
        .iter()
        .filter(|section| section.header == "lib")
        .any(|section| {
            section
                .lines
                .iter()
                .any(|(_, line)| toml_value(line, "doctest") == Some("false"))
        })
}

/// `name` from `[package]`.
pub(crate) fn package_name(manifest: &str) -> Option<String> {
    toml_sections(manifest)
        .iter()
        .find(|section| section.header == "package")
        .and_then(|section| {
            section
                .lines
                .iter()
                .find_map(|(_, line)| toml_value(line, "name"))
                .map(str::to_owned)
        })
}

/// Rule 1.12.
pub(crate) fn check_no_raw_path_deps(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for member in repo.members()? {
        let manifest = repo.read(&member.manifest)?;
        for (line, text) in raw_path_deps(&manifest) {
            findings.push(Finding {
                file: Some(format!("{}:{line}", display(&member.manifest))),
                message: format!(
                    "hand-written path dependency `{text}` — declare it once in the root [workspace.dependencies] and write `<crate>.workspace = true` here"
                ),
            });
        }
    }
    Ok(findings)
}

/// Rule 2.10.
pub(crate) fn check_internal_doctest_off(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for member in repo.members()? {
        if !member.is_lib(repo) {
            continue;
        }
        let manifest = repo.read(&member.manifest)?;
        if !declares_doctest_off(&manifest) {
            findings.push(Finding {
                file: Some(display(&member.manifest)),
                message: "internal library without `[lib] doctest = false` — it pays for a doctest binary nobody reads".to_owned(),
            });
        }
    }
    Ok(findings)
}

/// Rule 1.3.
pub(crate) fn check_dir_name_matches_crate(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for member in repo.members()? {
        // `tools/xtask` is the one documented exception (CONTRACT.md §2).
        if member.dir.starts_with(Path::new("tools")) {
            continue;
        }
        let manifest = repo.read(&member.manifest)?;
        let Some(name) = package_name(&manifest) else {
            findings.push(Finding {
                file: Some(display(&member.manifest)),
                message: "no `name` under [package]".to_owned(),
            });
            continue;
        };
        if name != member.dir_name {
            findings.push(Finding {
                file: Some(display(&member.manifest)),
                message: format!(
                    "package `{name}` lives in directory `{}` — a path and a `-p` flag must name the same crate",
                    member.dir_name
                ),
            });
        }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests;
