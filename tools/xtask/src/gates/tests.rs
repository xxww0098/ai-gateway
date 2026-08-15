use super::*;
use crate::fixture::Fixture;

#[test]
fn every_gate_is_registered_once_and_names_its_rule() {
    let mut names: Vec<&str> = GATES.iter().map(|gate| gate.name).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "duplicate gate name");

    for gate in GATES {
        assert!(!gate.rule.is_empty(), "{} has no rule number", gate.name);
        assert!(!gate.summary.is_empty(), "{} has no summary", gate.name);
    }
}

#[test]
fn the_four_load_bearing_gates_are_present() {
    // These are the ones CONTRACT.md's own self-check cannot replace.
    for required in [
        "no_orphan_modules",
        "file_ownership",
        "no_raw_path_deps",
        "build_override_pair",
    ] {
        assert!(
            GATES.iter().any(|gate| gate.name == required),
            "{required} is missing from the registry"
        );
    }
}

#[test]
fn a_clean_tree_reports_no_findings() {
    let fixture = Fixture::new("gates-clean");
    fixture
        .write(
            "CONTRACT.md",
            "## 3. 文件所有权\n\n| worker | 独占目录 |\n| --- | --- |\n| `alpha` | `crates/gw-x/**` |\n\n**协调者独占**：`Cargo.toml`、`CONTRACT.md`。\n\n## 4. next\n",
        )
        .member("gw-x", "");

    assert_eq!(run_all(&fixture.repo()).expect("run"), 0);
}

#[test]
fn findings_are_counted_across_gates() {
    let fixture = Fixture::new("gates-dirty");
    fixture
        .write(
            "CONTRACT.md",
            "## 3. 文件所有权\n\n| worker | 独占目录 |\n| --- | --- |\n| `alpha` | `crates/gw-x/**` |\n\n**协调者独占**：`Cargo.toml`、`CONTRACT.md`。\n\n## 4. next\n",
        )
        .member("gw-x", "")
        // One orphan (2.6) and one hand-written path dep (1.12).
        .write("crates/gw-x/src/orphan.rs", "")
        .write(
            "crates/gw-x/Cargo.toml",
            "[package]\nname = \"gw-x\"\n\n[lib]\ndoctest = false\n\n[dependencies]\nother = { path = \"../other\" }\n",
        );

    assert_eq!(run_all(&fixture.repo()).expect("run"), 2);
}
