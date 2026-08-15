//! Test-suite and file-size hygiene.
//!
//! * rule 2.9 (antipattern #7) — a test that reads an env var and `return`s
//!   when it is missing reports success without testing anything. Coverage
//!   becomes fiction and CI stays green through a broken integration. The two
//!   honest options are fail-loud with instructions, or `#[ignore = "needs X"]`.
//! * rule 1.10 — 1,000 lines in one file is the point where you stop and look.
//!   The whitelist below is a RATCHET: entries may be removed, never added.

use std::path::Path;

use anyhow::Result;

use crate::gates::Finding;
use crate::repo::{Repo, display};

/// Rule 1.10's threshold.
pub(crate) const MAX_FILE_LINES: usize = 1_000;

/// Files allowed to exceed [`MAX_FILE_LINES`] today.
///
/// RATCHET (rule 5.3): this list may only shrink. Adding an entry to make the
/// gate pass is the failure mode it exists to prevent — split the file instead.
/// It is empty because no file has ever needed to be on it.
pub(crate) const LONG_FILE_ALLOWLIST: &[&str] = &[];

/// How far after an `env::var` read a `return` still counts as a silent skip.
const SKIP_WINDOW: usize = 3;

/// Blank out the contents of string literals, keeping line and column numbers.
///
/// Without this the gate flags its own test fixtures — and every other file
/// that quotes example code. Known limits: a raw string's delimiters are
/// treated as ordinary quotes, which still blanks the body.
fn blank_string_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_string = false;

    for line in source.lines() {
        let mut blanked = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        let mut escaped = false;
        let mut in_comment = false;

        while let Some(current) = chars.next() {
            if in_comment {
                blanked.push(' ');
                continue;
            }
            if in_string {
                let was_escaped = escaped;
                escaped = current == '\\' && !was_escaped;
                // `\"` stays inside the literal; a bare `"` closes it.
                if current == '"' && !was_escaped {
                    in_string = false;
                    blanked.push('"');
                } else {
                    blanked.push(' ');
                }
                continue;
            }
            match current {
                '"' => {
                    in_string = true;
                    escaped = false;
                    blanked.push('"');
                }
                // A `'"'` char literal must not open a string.
                '\'' if chars.peek() == Some(&'"') => {
                    chars.next();
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                    }
                    blanked.push_str("   ");
                }
                '/' if chars.peek() == Some(&'/') => {
                    in_comment = true;
                    blanked.push(' ');
                }
                other => blanked.push(other),
            }
        }
        out.push(blanked);
    }

    out
}

/// Lines that look like "no env var, no test".
pub(crate) fn silent_skips(source: &str) -> Vec<(usize, String)> {
    let original: Vec<&str> = source.lines().collect();
    let blanked = blank_string_literals(source);
    let lines: Vec<&str> = blanked.iter().map(String::as_str).collect();
    let mut out = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !line.contains("env::var(") {
            continue;
        }
        let window = &lines[index..lines.len().min(index + 1 + SKIP_WINDOW)];
        let bails = window.iter().any(|line| has_return_keyword(line));
        // A test that fails loudly on the missing variable is exactly right.
        let loud = window.iter().any(|line| {
            line.contains("panic!") || line.contains("expect(") || line.contains("unwrap()")
        });

        if bails && !loud {
            out.push((index + 1, original[index].trim().to_owned()));
        }
    }

    out
}

/// `return` as a keyword, not as part of an identifier — `Err(_) => return,`
/// counts, `v.returns_something()` does not.
fn has_return_keyword(line: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find("return") {
        let before = rest[..at].chars().next_back();
        let after = rest[at + 6..].chars().next();
        let is_word_boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if is_word_boundary(before) && is_word_boundary(after) {
            return true;
        }
        rest = &rest[at + 6..];
    }
    false
}

/// Whether a file holds tests (and is therefore in scope for rule 2.9).
fn is_test_file(path: &Path, source: &str) -> bool {
    let shown = display(path);
    shown.contains("/tests/")
        || shown.ends_with("/tests.rs")
        || shown.ends_with("_tests.rs")
        || source.contains("#[test]")
        || source.contains("::test]")
}

/// Rule 2.9.
pub(crate) fn check_no_silent_test_skip(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    for member in repo.members()? {
        let mut files = repo.rust_files_under(&member.dir.join("src"))?;
        files.extend(repo.rust_files_under(&member.dir.join("tests"))?);

        for file in files {
            let source = repo.read(&file)?;
            if !is_test_file(&file, &source) {
                continue;
            }
            for (line, text) in silent_skips(&source) {
                findings.push(Finding {
                    file: Some(format!("{}:{line}", display(&file))),
                    message: format!(
                        "`{text}` looks like a test that returns when its environment is missing — that turns a skipped test into a passing one. Fail loudly with the fix, or mark it `#[ignore = \"needs ...\"]`"
                    ),
                });
            }
        }
    }

    Ok(findings)
}

/// Rule 1.10, ratcheted.
pub(crate) fn check_file_length(repo: &Repo) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut still_needed: Vec<&str> = Vec::new();

    for member in repo.members()? {
        let mut files = repo.rust_files_under(&member.dir.join("src"))?;
        files.extend(repo.rust_files_under(&member.dir.join("tests"))?);

        for file in files {
            let shown = display(&file);
            let lines = repo.read(&file)?.lines().count();
            let allowed = LONG_FILE_ALLOWLIST.contains(&shown.as_str());

            if lines > MAX_FILE_LINES && !allowed {
                findings.push(Finding {
                    file: Some(shown),
                    message: format!(
                        "{lines} lines (limit {MAX_FILE_LINES}) — split it into submodules; the allowlist in xtask is a ratchet and may not grow"
                    ),
                });
            } else if allowed && lines > MAX_FILE_LINES {
                still_needed.push(
                    LONG_FILE_ALLOWLIST
                        .iter()
                        .find(|entry| **entry == shown)
                        .expect("allowlist hit"),
                );
            }
        }
    }

    // The ratchet's other half: an entry that is no longer needed must go, or
    // the list silently re-authorizes the next file to reach that name.
    for entry in LONG_FILE_ALLOWLIST {
        if !still_needed.contains(entry) {
            findings.push(Finding {
                file: Some((*entry).to_owned()),
                message: "on the long-file allowlist but no longer over the limit (or gone) — remove the entry so the ratchet holds".to_owned(),
            });
        }
    }

    Ok(findings)
}

#[cfg(test)]
mod tests;
