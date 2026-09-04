//! `boot run` refuses a malformed artifact request before the boot
//! inputs resolve, so a bad `--observation-manifest` costs seconds
//! rather than the run it would have saved. Needs no installed title:
//! every case must die before the boot-started sentinel.

use std::path::PathBuf;
use std::process::Command;

use cellgov_compare::witnesses::{BOOT_STARTED_SENTINEL, TITLE_NOT_INSTALLED_SENTINEL};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "cellgov_run_game_preflight_{label}_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("write scratch file");
        p.to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Run `boot run` with `extra` and return `(exit ok, stderr)`. The
/// title selector is never resolved: every case dies first.
fn run_game(extra: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["boot", "run"])
        .args(["--title", "preflight-only", "--max-steps", "1"])
        .args(extra)
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov boot run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn assert_died_before_boot(stderr: &str) {
    assert!(
        !stderr.contains(BOOT_STARTED_SENTINEL) && !stderr.contains(TITLE_NOT_INSTALLED_SENTINEL),
        "the manifest must be refused before the boot inputs resolve:\n{stderr}"
    );
}

#[test]
fn a_malformed_manifest_is_refused_before_the_boot_starts() {
    let scratch = Scratch::new("malformed");
    let manifest = scratch.file(
        "bad.toml",
        "[[regions]]\nname = \"code\"\naddr = \"not-hex\"\nsize = \"0x10\"\n",
    );
    let (ok, stderr) = run_game(&[
        "--save-observation",
        &scratch.path("obs.json"),
        "--observation-manifest",
        &manifest,
    ]);
    assert!(!ok, "a malformed manifest must fail the run:\n{stderr}");
    assert!(
        stderr.contains("--observation-manifest: parse") && stderr.contains("not-hex"),
        "the error names the flag and the bad value:\n{stderr}"
    );
    assert!(
        stderr.contains("hex string like \"0x10000\""),
        "the error names the accepted form:\n{stderr}"
    );
    assert_died_before_boot(&stderr);
}

#[test]
fn a_missing_manifest_is_refused_before_the_boot_starts() {
    let scratch = Scratch::new("missing");
    let (ok, stderr) = run_game(&[
        "--save-observation",
        &scratch.path("obs.json"),
        "--observation-manifest",
        &scratch.path("absent.toml"),
    ]);
    assert!(!ok, "a missing manifest must fail the run:\n{stderr}");
    assert!(
        stderr.contains("--observation-manifest: read") && stderr.contains("absent.toml"),
        "the error names the flag and the path:\n{stderr}"
    );
    assert_died_before_boot(&stderr);
}

#[test]
fn a_manifest_without_a_save_target_is_refused() {
    let scratch = Scratch::new("no_target");
    let manifest = scratch.file(
        "ok.toml",
        "[[regions]]\nname = \"code\"\naddr = \"0x10000\"\nsize = 16\n",
    );
    let (ok, stderr) = run_game(&["--observation-manifest", &manifest]);
    assert!(!ok, "a manifest with nothing to save must fail:\n{stderr}");
    assert!(
        stderr.contains("--observation-manifest") && stderr.contains("--save-observation"),
        "the refusal names both flags:\n{stderr}"
    );
    assert_died_before_boot(&stderr);
}
