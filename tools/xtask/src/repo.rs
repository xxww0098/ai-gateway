//! Filesystem view of the workspace, shared by every gate.
//!
//! Deliberately dependency-free: `xtask` takes `anyhow` and nothing else, so
//! there is no TOML parser and no glob crate here. The gates only need to read
//! section headers and key/value lines out of manifests, which a line scanner
//! does honestly — and a line scanner cannot silently "fix up" a manifest the
//! way a lenient parser might.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Directories never walked: build output and version control.
const SKIP_DIRS: &[&str] = &["target", ".git"];

/// The `rust/` workspace root.
#[derive(Debug, Clone)]
pub(crate) struct Repo {
    root: PathBuf,
}

/// One workspace member.
#[derive(Debug, Clone)]
pub(crate) struct Member {
    /// Directory name, e.g. `gw-config`. Rule 1.3 makes this the crate name for
    /// everything under `crates/`.
    pub(crate) dir_name: String,
    /// Path relative to the repo root, e.g. `crates/gw-config`.
    pub(crate) dir: PathBuf,
    /// Path to `Cargo.toml`, relative to the repo root.
    pub(crate) manifest: PathBuf,
}

impl Member {
    /// Whether this member is a library (has `src/lib.rs`).
    pub(crate) fn is_lib(&self, repo: &Repo) -> bool {
        repo.absolute(&self.dir.join("src/lib.rs")).is_file()
    }
}

impl Repo {
    /// Find the workspace root by walking up from the current directory, then
    /// from this crate's own manifest directory.
    ///
    /// The marker is `CONTRACT.md` next to a `Cargo.toml`: `cargo xtask` may be
    /// invoked from anywhere inside the tree.
    pub(crate) fn discover() -> Result<Repo> {
        let from_cwd = std::env::current_dir().ok();
        let from_manifest = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

        for start in [from_cwd, from_manifest].into_iter().flatten() {
            if let Some(root) = ascend_to_root(&start) {
                return Ok(Repo { root });
            }
        }
        bail!(
            "could not locate the workspace root (looked for a directory holding both CONTRACT.md and Cargo.toml)"
        )
    }

    /// Wrap an already-known root. Only the fixture tree needs this: the real
    /// entry point is [`Repo::discover`].
    #[cfg(test)]
    pub(crate) fn at(root: impl Into<PathBuf>) -> Repo {
        Repo { root: root.into() }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path for a repo-relative one.
    pub(crate) fn absolute(&self, relative: &Path) -> PathBuf {
        self.root.join(relative)
    }

    /// Read a repo-relative file.
    pub(crate) fn read(&self, relative: &Path) -> Result<String> {
        let path = self.absolute(relative);
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    }

    /// Every workspace member, in a stable order.
    ///
    /// Enumerated from the filesystem (`crates/*`, `tools/*`) rather than from
    /// the root manifest's globs: cargo already fails loudly when a listed
    /// member is missing, and the ownership table is written against the
    /// directory layout.
    pub(crate) fn members(&self) -> Result<Vec<Member>> {
        let mut members = Vec::new();
        for parent in ["crates", "tools"] {
            let parent_dir = self.root.join(parent);
            if !parent_dir.is_dir() {
                continue;
            }
            let mut entries: Vec<_> = std::fs::read_dir(&parent_dir)
                .with_context(|| format!("reading {}", parent_dir.display()))?
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(std::fs::DirEntry::file_name);

            for entry in entries {
                let dir = entry.path();
                if !dir.is_dir() || !dir.join("Cargo.toml").is_file() {
                    continue;
                }
                let dir_name = entry.file_name().to_string_lossy().into_owned();
                let relative = PathBuf::from(parent).join(&dir_name);
                members.push(Member {
                    dir_name,
                    manifest: relative.join("Cargo.toml"),
                    dir: relative,
                });
            }
        }
        Ok(members)
    }

    /// Every file under `relative`, repo-relative and sorted, skipping build
    /// output. Returns an empty vec when the directory does not exist.
    pub(crate) fn files_under(&self, relative: &Path) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let start = self.absolute(relative);
        if start.is_dir() {
            collect_files(&start, &self.root, &mut out)?;
        } else if start.is_file() {
            out.push(relative.to_path_buf());
        }
        out.sort();
        Ok(out)
    }

    /// Every `*.rs` file under `relative`, repo-relative and sorted.
    pub(crate) fn rust_files_under(&self, relative: &Path) -> Result<Vec<PathBuf>> {
        Ok(self
            .files_under(relative)?
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect())
    }
}

fn ascend_to_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join("CONTRACT.md").is_file() && dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files(&path, root, out)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Repo-relative path as a `/`-separated string, for matching and reporting.
pub(crate) fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------------------
// Minimal TOML reading
// ---------------------------------------------------------------------------

/// A `[section.header]` and the lines under it, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TomlSection {
    /// Header without brackets, e.g. `profile.dev.build-override`. The empty
    /// string is the implicit top-of-file section.
    pub(crate) header: String,
    /// `(1-based line number, trimmed line)` for every non-blank, non-comment
    /// line in the section.
    pub(crate) lines: Vec<(usize, String)>,
}

/// Split a manifest into sections. Array-of-table headers (`[[bin]]`) keep
/// their inner name (`bin`).
pub(crate) fn toml_sections(source: &str) -> Vec<TomlSection> {
    let mut sections = vec![TomlSection {
        header: String::new(),
        lines: Vec::new(),
    }];

    for (index, raw) in source.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let header = header
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(header);
            sections.push(TomlSection {
                header: header.trim().to_owned(),
                lines: Vec::new(),
            });
        } else if let Some(section) = sections.last_mut() {
            section.lines.push((index + 1, line.to_owned()));
        }
    }

    sections
}

/// The value of a bare `key = "value"` / `key = value` line, if present.
pub(crate) fn toml_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (found, value) = line.split_once('=')?;
    if found.trim() != key {
        return None;
    }
    Some(value.trim().trim_matches('"').trim())
}

#[cfg(test)]
mod tests;
