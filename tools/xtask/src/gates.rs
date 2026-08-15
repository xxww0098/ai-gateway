//! The gate registry and its runner.
//!
//! Rule 5.5: a rule nobody can check WILL rot — so every convention this
//! workspace relies on ends up here, and `cargo xtask ci` runs the lot.
//! Rule 5.2's finding was the other half: four passing checks existed and
//! nothing ever called them, which is why this is one command with one exit
//! code rather than a directory of scripts.
//!
//! Output goes to stderr on purpose: `print_stdout` is a workspace lint, and a
//! checker's diagnostics belong on the same stream as the compiler's.

pub(crate) mod hygiene;
pub(crate) mod manifests;
pub(crate) mod modules;
pub(crate) mod ownership;
pub(crate) mod profiles;

use anyhow::Result;

use crate::repo::Repo;

/// One violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    /// Repo-relative path, optionally `path:line`.
    pub(crate) file: Option<String>,
    /// What is wrong and what to do about it.
    pub(crate) message: String,
}

/// A check, its rule number, and what it is protecting.
pub(crate) struct Gate {
    pub(crate) name: &'static str,
    pub(crate) rule: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) run: fn(&Repo) -> Result<Vec<Finding>>,
}

/// Every gate, highest value first.
pub(crate) const GATES: &[Gate] = &[
    Gate {
        name: "no_orphan_modules",
        rule: "2.6",
        summary: "every .rs file is reachable from a crate root",
        run: modules::check_no_orphan_modules,
    },
    Gate {
        name: "file_ownership",
        rule: "CONTRACT §3",
        summary: "the ownership table matches the tree, with no gaps or overlaps",
        run: ownership::check_file_ownership,
    },
    Gate {
        name: "no_raw_path_deps",
        rule: "1.12",
        summary: "members use workspace dependencies, not hand-written paths",
        run: manifests::check_no_raw_path_deps,
    },
    Gate {
        name: "build_override_pair",
        rule: "3.3",
        summary: "per-package profile overrides come with build-override",
        run: profiles::check_build_override_pair,
    },
    Gate {
        name: "no_wildcard_opt",
        rule: "3.2",
        summary: "no [profile.*.package.\"*\"]",
        run: profiles::check_no_wildcard_opt,
    },
    Gate {
        name: "internal_doctest_off",
        rule: "2.10",
        summary: "internal libraries disable doctests",
        run: manifests::check_internal_doctest_off,
    },
    Gate {
        name: "dir_name_matches_crate",
        rule: "1.3",
        summary: "directory name equals crate name",
        run: manifests::check_dir_name_matches_crate,
    },
    Gate {
        name: "no_silent_test_skip",
        rule: "2.9",
        summary: "tests fail loudly or are #[ignore]d, never silently skipped",
        run: hygiene::check_no_silent_test_skip,
    },
    Gate {
        name: "file_length",
        rule: "1.10",
        summary: "no file over 1,000 lines outside the shrinking allowlist",
        run: hygiene::check_file_length,
    },
];

/// Run every gate, print a report to stderr, and return the total number of
/// findings (0 = clean).
pub(crate) fn run_all(repo: &Repo) -> Result<usize> {
    eprintln!(
        "xtask ci — {} gates over {}",
        GATES.len(),
        repo.root().display()
    );

    let mut total = 0;
    for gate in GATES {
        let findings = (gate.run)(repo)?;
        total += findings.len();

        if findings.is_empty() {
            eprintln!("  ok    {:<24} rule {}", gate.name, gate.rule);
            continue;
        }

        eprintln!(
            "  FAIL  {:<24} rule {} — {} finding(s)",
            gate.name,
            gate.rule,
            findings.len()
        );
        for finding in &findings {
            match &finding.file {
                Some(file) => eprintln!("          {file}: {}", finding.message),
                None => eprintln!("          {}", finding.message),
            }
        }
    }

    if total == 0 {
        eprintln!("all gates clean");
    } else {
        eprintln!("{total} finding(s) across {} gates", GATES.len());
    }
    Ok(total)
}

/// Print the registry without running anything.
pub(crate) fn list() {
    for gate in GATES {
        eprintln!("{:<24} rule {:<12} {}", gate.name, gate.rule, gate.summary);
    }
}

#[cfg(test)]
mod tests;
