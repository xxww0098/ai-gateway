// Rule 5.3 ratchet — the bin is its own crate, so the lib's deny does not
// reach it. The three lines below are the whole binary (rule 1.5).
#![deny(clippy::todo, clippy::unimplemented)]

fn main() -> anyhow::Result<()> {
    gw_server::run()
}
