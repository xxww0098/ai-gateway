//! A throwaway workspace on disk, for gates that must be tested against real
//! files rather than strings.
//!
//! Test-only. No `tempfile` dependency: `xtask` takes `anyhow` and nothing
//! else, and a fixture directory keyed by process id and test name is unique
//! enough for a test binary that owns its own process.

use std::path::PathBuf;

use crate::repo::Repo;

/// A directory tree that deletes itself.
pub(crate) struct Fixture {
    root: PathBuf,
}

impl Fixture {
    /// Create an empty fixture. `name` must be unique within the test binary.
    pub(crate) fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir()
            .join("cpa-xtask-fixtures")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");

        let fixture = Fixture { root };
        // Every fixture is a workspace: `Repo::at` does not check, but the
        // gates read these two files.
        fixture
            .write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n")
            .write("CONTRACT.md", "## 3. 文件所有权\n\n## 4. next\n");
        fixture
    }

    /// Write a file, creating parent directories.
    pub(crate) fn write(&self, relative: &str, contents: &str) -> &Fixture {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture parent");
        }
        std::fs::write(&path, contents).expect("write fixture file");
        self
    }

    /// A minimal library member named `name`.
    pub(crate) fn member(&self, name: &str, lib_rs: &str) -> &Fixture {
        self.write(
            &format!("crates/{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\n\n[lib]\ndoctest = false\n"),
        )
        .write(&format!("crates/{name}/src/lib.rs"), lib_rs)
    }

    pub(crate) fn repo(&self) -> Repo {
        Repo::at(&self.root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
