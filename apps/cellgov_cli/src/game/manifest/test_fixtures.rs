//! Shared `#[cfg(test)]` scaffolding: a self-cleaning temp directory
//! and the canonical manifest TOML strings used across loader and
//! registry tests.

use std::path::{Path, PathBuf};

pub(super) struct TmpDir(PathBuf);

impl TmpDir {
    pub(super) fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cellgov_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) const PROCESS_EXIT_TOML: &str = r#"
[title]
content_id = "NPAA00001"
short_name = "proc-exit-fixture"
display_name = "Process-exit checkpoint fixture"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "process-exit"
"#;

pub(super) const FIRST_RSX_WRITE_TOML: &str = r#"
[title]
content_id = "NPAA00002"
short_name = "rsx-write-fixture"
display_name = "First-RSX-write checkpoint fixture"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "first-rsx-write"
"#;

pub(super) const PC_TOML: &str = r#"
[title]
content_id = "NPAA00003"
short_name = "pcstop"
display_name = "PC-checkpoint test title"
eboot_candidates = ["EBOOT.elf"]

[checkpoint]
kind = "pc"
pc = "0x10381ce8"
"#;
