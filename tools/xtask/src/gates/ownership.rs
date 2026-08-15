//! `CONTRACT.md` §3 — the file-ownership table, checked against reality.
//!
//! Rule 5.5: a rule nobody can check WILL rot. The ownership table is the only
//! thing that makes nine workers in one worktree safe, and it is prose — so it
//! is exactly the rule most likely to drift out of sync with the tree.
//!
//! Three failures are reported:
//!
//! 1. a pattern that matches nothing (the table describes a tree that no longer
//!    exists);
//! 2. a file no pattern claims (a worker created something nobody owns);
//! 3. a file two different workers claim (the table promises isolation it does
//!    not deliver).
//!
//! Ownership of a file the table does not name directly is INHERITED through
//! the module graph: `src/claude/tests.rs` belongs to whoever owns
//! `src/claude.rs`, because that file is the only thing that can reach it. The
//! table therefore lists entry points, and the compiler's own reachability
//! settles the rest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::gates::{Finding, modules};
use crate::repo::{Repo, display};

/// Owner label used for the "协调者独占" block, which overrides worker rows.
pub(crate) const COORDINATOR: &str = "协调者";

/// Directory names whose contents are fixtures, not owned source.
const FIXTURE_DIRS: &[&str] = &["testdata", "fixtures", "snapshots"];

/// Files at the repo root that are tooling artifacts rather than owned source.
const UNOWNED_ARTIFACTS: &[&str] = &["Cargo.lock", ".gitignore"];

/// One `owner -> pattern` claim parsed out of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Claim {
    pub(crate) owner: String,
    pub(crate) pattern: String,
}

impl Claim {
    pub(crate) fn is_coordinator(&self) -> bool {
        self.owner == COORDINATOR
    }
}

/// Parse §3 of `CONTRACT.md`.
///
/// Table rows contribute `worker -> patterns`; the prose block introduced by
/// "协调者独占" contributes coordinator claims.
pub(crate) fn parse_claims(markdown: &str) -> Vec<Claim> {
    let section = section_3(markdown);
    let mut claims = Vec::new();
    let mut in_coordinator_block = false;

    for line in section.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('|') {
            let cells: Vec<&str> = trimmed.trim_matches('|').split('|').collect();
            if cells.len() < 2 {
                continue;
            }
            let owner = backticked(cells[0]);
            let (Some(owner), patterns) = (owner.first().cloned(), backticked(cells[1])) else {
                continue;
            };
            for pattern in patterns.into_iter().filter(|p| is_path_like(p)) {
                claims.push(Claim {
                    owner: owner.clone(),
                    pattern,
                });
            }
            continue;
        }

        if trimmed.contains("协调者独占") {
            in_coordinator_block = true;
        }
        if in_coordinator_block {
            for pattern in backticked(trimmed).into_iter().filter(|p| is_path_like(p)) {
                claims.push(Claim {
                    owner: COORDINATOR.to_owned(),
                    pattern,
                });
            }
        }
    }

    claims
}

/// The `## 3. ...` section, up to the next `## ` heading.
///
/// Scanned line-wise rather than by substring search so a table at the very
/// start of the file (as in the gate's own fixtures) is found too.
fn section_3(markdown: &str) -> &str {
    let mut start = None;
    let mut offset = 0;

    for line in markdown.split_inclusive('\n') {
        match start {
            None => {
                if line.starts_with("## 3.") {
                    start = Some(offset);
                }
            }
            Some(from) if line.starts_with("## ") && offset > from => {
                return &markdown[from..offset];
            }
            Some(_) => {}
        }
        offset += line.len();
    }

    start.map_or("", |from| &markdown[from..])
}

/// Every `` `token` `` in a line.
fn backticked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let token = after[..close].trim();
        if !token.is_empty() {
            out.push(token.to_owned());
        }
        rest = &after[close + 1..];
    }
    out
}

fn is_path_like(token: &str) -> bool {
    token.contains('/') || token.ends_with(".toml") || token.ends_with(".md")
}

/// Normalize a claim into a repo-relative pattern: the table writes some paths
/// from the git root (`rust/docs/**`) and some from the workspace root
/// (`crates/gw-config/**`).
pub(crate) fn normalize(pattern: &str) -> String {
    pattern
        .trim()
        .trim_start_matches("./")
        .strip_prefix("rust/")
        .unwrap_or(pattern.trim())
        .to_owned()
}

/// Match a path against one pattern.
///
/// Supports `**` (any number of segments), `*` (any characters within one
/// segment) and `{a,b}` alternatives — the three forms CONTRACT.md actually
/// uses.
pub(crate) fn glob_match(pattern: &str, path: &str) -> bool {
    expand_braces(pattern)
        .into_iter()
        .any(|expanded| match_segments(&split(&expanded), &split(path)))
}

fn split(text: &str) -> Vec<&str> {
    text.split('/').filter(|s| !s.is_empty()).collect()
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let Some(close) = pattern[open..].find('}').map(|at| at + open) else {
        return vec![pattern.to_owned()];
    };

    let (prefix, suffix) = (&pattern[..open], &pattern[close + 1..]);
    pattern[open + 1..close]
        .split(',')
        .flat_map(|alt| expand_braces(&format!("{prefix}{}{suffix}", alt.trim())))
        .collect()
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            if rest.is_empty() {
                // A trailing `**` claims the whole subtree, but `crates/x/**`
                // must not claim `crates/x` itself being a file.
                return true;
            }
            (0..=path.len()).any(|skip| match_segments(rest, &path[skip..]))
        }
        Some((head, rest)) => match path.split_first() {
            Some((first, tail)) if match_one(head, first) => match_segments(rest, tail),
            _ => false,
        },
    }
}

/// Segment match with `*` wildcards.
fn match_one(pattern: &str, segment: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == segment;
    }

    let mut rest = segment;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match index {
            0 => match rest.strip_prefix(part) {
                Some(tail) => rest = tail,
                None => return false,
            },
            _ if index == parts.len() - 1 => return rest.ends_with(part),
            _ => match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            },
        }
    }
    true
}

/// Files whose ownership the table is expected to settle.
fn owned_universe(repo: &Repo) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for relative in ["crates", "tools", "migrations", "docs"] {
        files.extend(repo.files_under(Path::new(relative))?);
    }
    for name in ["Cargo.toml", "rust-toolchain.toml", "CONTRACT.md"] {
        let path = PathBuf::from(name);
        if repo.absolute(&path).is_file() {
            files.push(path);
        }
    }
    let cargo_config = PathBuf::from(".cargo/config.toml");
    if repo.absolute(&cargo_config).is_file() {
        files.push(cargo_config);
    }

    files.retain(|file| {
        let shown = display(file);
        !UNOWNED_ARTIFACTS.contains(&shown.as_str())
            && !file
                .components()
                .any(|part| FIXTURE_DIRS.contains(&part.as_os_str().to_string_lossy().as_ref()))
    });
    files.sort();
    files.dedup();
    Ok(files)
}

/// Direct pattern owners of one path, coordinator claims taking precedence.
fn direct_owners<'a>(claims: &'a [Claim], path: &str) -> Vec<&'a Claim> {
    let matched: Vec<&Claim> = claims
        .iter()
        .filter(|claim| glob_match(&normalize(&claim.pattern), path))
        .collect();

    if matched.iter().any(|claim| claim.is_coordinator()) {
        return matched.into_iter().filter(|c| c.is_coordinator()).collect();
    }
    matched
}

/// The gate.
pub(crate) fn check_file_ownership(repo: &Repo) -> Result<Vec<Finding>> {
    let contract = repo
        .read(Path::new("CONTRACT.md"))
        .context("CONTRACT.md is the source of truth for this gate")?;
    let claims = parse_claims(&contract);
    let mut findings = Vec::new();

    if claims.is_empty() {
        findings.push(Finding {
            file: Some("CONTRACT.md".to_owned()),
            message: "no ownership claims parsed out of §3 — the table moved or changed shape, and this gate is now checking nothing".to_owned(),
        });
        return Ok(findings);
    }

    let universe = owned_universe(repo)?;
    let shown: Vec<String> = universe.iter().map(|p| display(p)).collect();

    // 1. Stale patterns.
    for claim in &claims {
        let pattern = normalize(&claim.pattern);
        if !shown.iter().any(|path| glob_match(&pattern, path)) {
            findings.push(Finding {
                file: Some("CONTRACT.md".to_owned()),
                message: format!(
                    "§3 gives `{}` to `{}`, but nothing in the tree matches it",
                    claim.pattern, claim.owner
                ),
            });
        }
    }

    // 2 + 3. Resolve every file, directly or by inheritance through the module
    // graph, then look for gaps and conflicts.
    let mut graphs = BTreeMap::new();
    for member in repo.members()? {
        graphs.insert(member.dir.clone(), modules::graph_of(repo, &member)?);
    }

    for (file, path) in universe.iter().zip(&shown) {
        let owners = direct_owners(&claims, path);

        let distinct: Vec<&str> = {
            let mut names: Vec<&str> = owners.iter().map(|c| c.owner.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            names
        };

        if distinct.len() > 1 {
            findings.push(Finding {
                file: Some(path.clone()),
                message: format!(
                    "claimed by {} at once ({}) — §3 promises exclusive ownership",
                    distinct.len(),
                    distinct.join(", ")
                ),
            });
            continue;
        }
        if !distinct.is_empty() {
            continue;
        }

        if let Some(inherited) = inherited_owner(&graphs, &claims, file) {
            let _ = inherited;
            continue;
        }

        findings.push(Finding {
            file: Some(path.clone()),
            message: "no owner in CONTRACT.md §3, and no module ancestor has one either — add it to the table or to an existing row".to_owned(),
        });
    }

    Ok(findings)
}

/// Follow the module graph upwards until an ancestor has a direct owner.
fn inherited_owner(
    graphs: &BTreeMap<PathBuf, modules::ModuleGraph>,
    claims: &[Claim],
    file: &Path,
) -> Option<String> {
    for graph in graphs.values() {
        let owner = graph.ancestor(file, |ancestor| {
            !direct_owners(claims, &display(ancestor)).is_empty()
        });
        if let Some(ancestor) = owner {
            return direct_owners(claims, &display(&ancestor))
                .first()
                .map(|claim| claim.owner.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests;
