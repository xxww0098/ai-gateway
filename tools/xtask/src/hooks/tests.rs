use super::*;

#[test]
fn the_hook_runs_the_gates_from_the_workspace_root() {
    assert!(PRE_COMMIT.starts_with("#!/bin/sh\n"));
    // `set -e` is what turns a failing gate into a rejected commit; without it
    // the hook reports the failure and commits anyway.
    assert!(PRE_COMMIT.contains("set -e"));
    assert!(PRE_COMMIT.contains("cargo xtask ci"));
    // The gates resolve paths relative to the workspace root, and a commit can
    // be made from any subdirectory.
    assert!(PRE_COMMIT.contains("git rev-parse --show-toplevel"));
}
