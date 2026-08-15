//! Machine-checkable gates for rust/CONTRACT.md and docs/rust-engineering.md.
//!
//! Rule 5.5: a rule nobody can check WILL rot. Every convention this workspace
//! relies on must end up as a check here, wired into a commit hook — rule 5.2's
//! finding was that four passing checks existed and nothing ever called them.
//!
//! ```text
//! cargo xtask ci      run every gate; exit 1 if any finding
//! cargo xtask gates   list the gates and the rules they enforce
//! cargo xtask hooks   print the pre-commit hook that runs `cargo xtask ci`
//! ```
//!
//! OWNER: worker `platform`.

#![deny(clippy::todo, clippy::unimplemented)]

#[cfg(test)]
mod fixture;
mod gates;
mod hooks;
mod repo;

use anyhow::Result;

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "ci" => {
            let repo = repo::Repo::discover()?;
            if gates::run_all(&repo)? > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        "gates" => {
            gates::list();
            Ok(())
        }
        "hooks" => {
            hooks::print_pre_commit();
            Ok(())
        }
        _ => {
            eprintln!("usage: cargo xtask <ci|gates|hooks>");
            std::process::exit(2);
        }
    }
}
