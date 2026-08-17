//! The commit hook that makes the gates matter.
//!
//! Rule 5.2's finding was not "there were no checks" — it was that four
//! passing checks existed and nothing ever called them. A gate nobody runs is
//! documentation with a compile step.
//!
//! This prints the hook rather than writing it: installing a hook mutates the
//! developer's `.git/` (outside this workspace, and outside the ownership
//! table), which is the operator's call, not a build tool's.

/// The hook body, ready to paste into `.git/hooks/pre-commit`.
pub(crate) const PRE_COMMIT: &str = r#"#!/bin/sh
# AI-GateWay rust/ gates — see rust/CONTRACT.md §7 and tools/xtask.
set -e
cd "$(git rev-parse --show-toplevel)/rust"
cargo xtask ci
cargo fmt --check
"#;

/// Print the hook and the one-liner that installs it.
pub(crate) fn print_pre_commit() {
    eprintln!("{PRE_COMMIT}");
    eprintln!("install with:");
    eprintln!(
        "  cargo xtask hooks 2>&1 | sed -n '1,/^$/p' > \"$(git rev-parse --git-dir)/hooks/pre-commit\""
    );
    eprintln!("  chmod +x \"$(git rev-parse --git-dir)/hooks/pre-commit\"");
}

#[cfg(test)]
mod tests;
