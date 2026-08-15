//! Rule 2.6 — every `.rs` file must be reachable from a crate root.
//!
//! This is the one defect class review cannot see: an unreachable file does not
//! error, does not warn, and is never compiled — so the tests inside it silently
//! stop existing and coverage becomes fiction.
//!
//! The module graph built here is also what [`super::ownership`] uses to decide
//! who owns a file the CONTRACT table does not name directly.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::gates::Finding;
use crate::repo::{Member, Repo, display, toml_sections, toml_value};

/// A `mod name;` declaration pointing at another file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModDecl {
    pub(crate) name: String,
    /// The `#[path = "..."]` override, when one is attached.
    pub(crate) path_attr: Option<String>,
    pub(crate) line: usize,
}

/// Which file declared which, for a single member crate.
#[derive(Debug, Default)]
pub(crate) struct ModuleGraph {
    /// Crate roots, as [`crate_roots`] resolves them: every Cargo target of the
    /// member, whether the manifest declares it or Cargo discovers it.
    pub(crate) roots: BTreeSet<PathBuf>,
    /// `child -> parent that declared it`.
    pub(crate) parents: BTreeMap<PathBuf, PathBuf>,
}

impl ModuleGraph {
    /// Every file the compiler can reach, roots included.
    pub(crate) fn reachable(&self) -> BTreeSet<PathBuf> {
        self.roots
            .iter()
            .chain(self.parents.keys())
            .cloned()
            .collect()
    }

    /// Walk up to the nearest ancestor satisfying `predicate`.
    pub(crate) fn ancestor(
        &self,
        file: &Path,
        predicate: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        let mut current = self.parents.get(file);
        let mut guard = 0;
        while let Some(parent) = current {
            if predicate(parent) {
                return Some(parent.clone());
            }
            // Module trees are shallow; the counter only exists so a malformed
            // graph cannot spin forever.
            guard += 1;
            if guard > 64 {
                return None;
            }
            current = self.parents.get(parent);
        }
        None
    }
}

/// Extract the external module declarations from one source file.
///
/// Inline modules (`mod tests { .. }`) declare no file and are skipped —
/// which is also what rule 2.2 forbids writing in the first place.
pub(crate) fn parse_mod_decls(source: &str) -> Vec<ModDecl> {
    let mut decls = Vec::new();
    let mut pending_path: Option<String> = None;

    for (index, raw) in source.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#[") || line.starts_with("#![") {
            if let Some(path) = path_attribute(line) {
                pending_path = Some(path);
            }
            continue;
        }

        if let Some(name) = mod_name(line) {
            decls.push(ModDecl {
                name,
                path_attr: pending_path.take(),
                line: index + 1,
            });
        }
        // Any other statement ends the attribute run.
        pending_path = None;
    }

    decls
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// `#[path = "foo.rs"]` -> `foo.rs`.
fn path_attribute(line: &str) -> Option<String> {
    let inner = line.trim_start_matches("#[").trim_end_matches(']');
    let (key, value) = inner.split_once('=')?;
    if key.trim() != "path" {
        return None;
    }
    Some(value.trim().trim_matches('"').to_owned())
}

/// `pub(crate) mod foo;` -> `foo`. Returns `None` for inline modules.
fn mod_name(line: &str) -> Option<String> {
    // Strip any visibility qualifier: bare `pub`, or `pub(..)` in any of its
    // spellings (`crate`, `super`, `in crate::x`).
    let rest = match line.strip_prefix("pub") {
        Some(after) if after.starts_with('(') => after
            .find(')')
            .map(|close| &after[close + 1..])
            .unwrap_or(after),
        Some(after) => after,
        None => line,
    }
    .trim_start();

    let name = rest.strip_prefix("mod ")?.trim();
    let name = name.strip_suffix(';')?.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name.to_owned())
}

/// Where a declaration could live, most specific first.
///
/// Mirrors rustc: `#[path]` on a non-inline module is relative to the directory
/// of the declaring file; otherwise a "mod-rs" file (`lib.rs` / `main.rs` /
/// `mod.rs`) owns its own directory while any other file owns a subdirectory
/// named after itself. `is_crate_root` covers the third case — an integration
/// test root such as `tests/billing.rs` resolves siblings, exactly like
/// `main.rs` does.
pub(crate) fn candidate_paths(
    decl: &ModDecl,
    declaring_file: &Path,
    is_crate_root: bool,
) -> Vec<PathBuf> {
    let dir = declaring_file.parent().unwrap_or(Path::new(""));

    if let Some(path) = &decl.path_attr {
        return vec![dir.join(path)];
    }

    let stem = declaring_file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = if is_crate_root || matches!(stem.as_str(), "lib" | "main" | "mod") {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };

    vec![
        base.join(format!("{}.rs", decl.name)),
        base.join(&decl.name).join("mod.rs"),
    ]
}

/// Cargo target sections. [`toml_sections`] keeps the inner name of an
/// array-of-table header, so `[[test]]` arrives here as `test`.
const TARGET_SECTIONS: &[&str] = &["lib", "bin", "test", "bench", "example"];

/// What one member manifest says about its own targets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ManifestTargets {
    /// Package-relative `path = "..."` of every explicitly declared target, in
    /// file order.
    pub(crate) paths: Vec<String>,
    /// `[package] autolib` — discovery of `src/lib.rs`.
    pub(crate) autolib: bool,
    /// `[package] autobins` — discovery of `src/main.rs` and `src/bin/**`.
    pub(crate) autobins: bool,
    /// `[package] autotests` — discovery of `tests/**`.
    pub(crate) autotests: bool,
}

impl Default for ManifestTargets {
    fn default() -> ManifestTargets {
        // Cargo's own defaults: everything is discovered unless switched off.
        ManifestTargets {
            paths: Vec::new(),
            autolib: true,
            autobins: true,
            autotests: true,
        }
    }
}

/// Read the explicit targets out of a member manifest.
///
/// `path` is only read inside a target section, so the `path = "../x"` of a
/// path dependency cannot be mistaken for a crate root.
pub(crate) fn parse_manifest_targets(source: &str) -> ManifestTargets {
    let mut targets = ManifestTargets::default();

    for section in toml_sections(source) {
        match section.header.as_str() {
            "package" => {
                for (_, line) in &section.lines {
                    for (key, flag) in [
                        ("autolib", &mut targets.autolib),
                        ("autobins", &mut targets.autobins),
                        ("autotests", &mut targets.autotests),
                    ] {
                        if let Some(value) = toml_value(line, key) {
                            *flag = value != "false";
                        }
                    }
                }
            }
            header if TARGET_SECTIONS.contains(&header) => {
                for (_, line) in &section.lines {
                    if let Some(path) = toml_value(line, "path") {
                        targets.paths.push(path.to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    targets
}

/// `<dir>/*.rs` plus `<dir>/*/main.rs` — the shape Cargo discovers under both
/// `tests/` and `src/bin/`.
///
/// The second half is the one that is easy to forget: `tests/panel/main.rs` is
/// built as the `panel` test binary with no manifest entry at all — checked
/// against cargo 1.97.1, the pin in `rust-toolchain.toml`. A `mod.rs` in that
/// position is NOT a root: `tests/common/mod.rs` is alive only because a root
/// declares it (rules 2.3 / 2.8).
fn discovered_roots(repo: &Repo, dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(repo
        .rust_files_under(dir)?
        .into_iter()
        .filter(|path| {
            let parent = path.parent();
            parent == Some(dir)
                || (path.ends_with("main.rs") && parent.and_then(Path::parent) == Some(dir))
        })
        .collect())
}

/// Every crate root of one member, resolved the way Cargo resolves targets.
///
/// Auto-discovery is only half the story. A manifest may declare a target
/// explicitly with its own `path`, and then THAT file is the crate root —
/// `gw-panel` declares `[[test]] path = "tests/panel/main.rs"` for its single
/// integration binary (rule 2.8). Assuming discovery covers everything makes
/// every file under such a target look unreachable, and a gate that cries wolf
/// gets muted along with the real orphans it catches (rules 5.2 / 5.5).
pub(crate) fn crate_roots(repo: &Repo, member: &Member) -> Result<BTreeSet<PathBuf>> {
    let mut roots = BTreeSet::new();
    let targets = parse_manifest_targets(&repo.read(&member.manifest).unwrap_or_default());

    let insert = |path: PathBuf, roots: &mut BTreeSet<PathBuf>| {
        if repo.absolute(&path).is_file() {
            roots.insert(path);
        }
    };

    // Explicit targets first: their `path` is the crate root, wherever it
    // points — including outside `src/` and `tests/`.
    for path in &targets.paths {
        insert(member.dir.join(path), &mut roots);
    }

    if targets.autolib {
        insert(member.dir.join("src/lib.rs"), &mut roots);
    }
    if targets.autobins {
        insert(member.dir.join("src/main.rs"), &mut roots);
        roots.extend(discovered_roots(repo, &member.dir.join("src/bin"))?);
    }
    if targets.autotests {
        roots.extend(discovered_roots(repo, &member.dir.join("tests"))?);
    }

    Ok(roots)
}

/// Build the module graph for one member crate.
pub(crate) fn graph_of(repo: &Repo, member: &Member) -> Result<ModuleGraph> {
    let mut graph = ModuleGraph {
        roots: crate_roots(repo, member)?,
        ..ModuleGraph::default()
    };

    let mut queue: VecDeque<PathBuf> = graph.roots.iter().cloned().collect();
    while let Some(file) = queue.pop_front() {
        let Ok(source) = repo.read(&file) else {
            continue;
        };
        for decl in parse_mod_decls(&source) {
            let Some(child) = candidate_paths(&decl, &file, graph.roots.contains(&file))
                .into_iter()
                .find(|candidate| repo.absolute(candidate).is_file())
            else {
                // A declaration with no file is rustc's problem, not ours: it
                // is a hard compile error, so it can never reach review.
                continue;
            };
            if graph.roots.contains(&child) || graph.parents.contains_key(&child) {
                continue;
            }
            graph.parents.insert(child.clone(), file.clone());
            queue.push_back(child);
        }
    }

    Ok(graph)
}

/// The gate: report every `.rs` file no crate root can reach.
pub(crate) fn check_no_orphan_modules(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    for member in repo.members()? {
        let graph = graph_of(repo, &member)?;
        let reachable = graph.reachable();

        let mut present = repo.rust_files_under(&member.dir.join("src"))?;
        present.extend(repo.rust_files_under(&member.dir.join("tests"))?);

        for file in present {
            if reachable.contains(&file) {
                continue;
            }
            findings.push(Finding {
                file: Some(display(&file)),
                message: format!(
                    "unreachable from any crate root: nothing declares it, so it is never compiled ({} tests inside it do not exist). Add a `mod` declaration, or delete the file.",
                    count_tests(&repo.read(&file).unwrap_or_default()),
                ),
            });
        }
    }

    Ok(findings)
}

/// `#[test]` / `#[tokio::test]` occurrences — the concrete cost of an orphan.
fn count_tests(source: &str) -> usize {
    source
        .lines()
        .filter(|line| {
            let line = line.trim();
            line == "#[test]" || line.ends_with("::test]")
        })
        .count()
}

#[cfg(test)]
mod tests;
