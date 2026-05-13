//! Integration tests for `cellgov_firmware install` against a real
//! PS3UPDAT.PUP.
//!
//! # Configuration
//!
//! - `CELLGOV_PS3UPDAT_PUP` points at the PUP file. Unset or
//!   non-existent skips silently.
//! - `CELLGOV_REQUIRE_PUP=1` makes a missing PUP fail instead.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries fixture-absent diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::PathBuf;
use std::process::Command;

const ENV_PUP: &str = "CELLGOV_PS3UPDAT_PUP";

/// Path from `CELLGOV_PS3UPDAT_PUP`, gated on the file existing.
fn locate_pup() -> Option<PathBuf> {
    let env_path = std::env::var(ENV_PUP).ok()?;
    let p = PathBuf::from(env_path);
    p.is_file().then_some(p)
}

fn require_pup(test_name: &str) -> Option<PathBuf> {
    if let Some(p) = locate_pup() {
        return Some(p);
    }
    if std::env::var_os("CELLGOV_REQUIRE_PUP").is_some() {
        panic!("CELLGOV_REQUIRE_PUP set but {ENV_PUP} is unset or points at a non-existent file");
    }
    eprintln!(
        "cellgov_firmware install integration ({test_name}): skipping \
         (set {ENV_PUP}=<absolute PUP path> to run)"
    );
    None
}

fn fresh_temp_dir(stem: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cellgov_firmware_install_{stem}_31b2"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn install_with_empty_output_dir_succeeds_and_populates_sys_external() {
    let Some(pup) = require_pup("happy_path") else {
        return;
    };
    let output = fresh_temp_dir("happy");
    let bin = env!("CARGO_BIN_EXE_cellgov_firmware");
    let result = Command::new(bin)
        .arg("install")
        .arg(&pup)
        .arg("--output")
        .arg(&output)
        .output()
        .expect("spawn cellgov_firmware install");
    assert!(
        result.status.success(),
        "install failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    let sys_external = output.join("sys").join("external");
    assert!(
        sys_external.is_dir(),
        "expected {} to exist after install",
        sys_external.display(),
    );
    let any_module = std::fs::read_dir(&sys_external)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().to_lowercase();
            s.ends_with(".prx") || s.ends_with(".sprx") || s.ends_with(".self")
        });
    assert!(
        any_module,
        "expected at least one .prx / .sprx / .self in {}",
        sys_external.display(),
    );
    let _ = std::fs::remove_dir_all(&output);
}

#[test]
fn install_refuses_non_empty_output_dir_without_force() {
    let Some(pup) = require_pup("refuse_overwrite") else {
        return;
    };
    let output = fresh_temp_dir("refuse");
    std::fs::write(output.join("decoy.txt"), b"existing").unwrap();

    let bin = env!("CARGO_BIN_EXE_cellgov_firmware");
    let result = Command::new(bin)
        .arg("install")
        .arg(&pup)
        .arg("--output")
        .arg(&output)
        .output()
        .expect("spawn cellgov_firmware install");
    assert!(
        !result.status.success(),
        "expected install to refuse non-empty dir without --force\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("exists and is non-empty"),
        "expected refuse-overwrite message in stderr, got:\n{stderr}",
    );
    let _ = std::fs::remove_dir_all(&output);
}

#[test]
fn install_force_flag_allows_non_empty_output_dir() {
    let Some(pup) = require_pup("force_flag") else {
        return;
    };
    let output = fresh_temp_dir("force");
    std::fs::write(output.join("decoy.txt"), b"existing").unwrap();

    let bin = env!("CARGO_BIN_EXE_cellgov_firmware");
    let result = Command::new(bin)
        .arg("install")
        .arg(&pup)
        .arg("--output")
        .arg(&output)
        .arg("--force")
        .output()
        .expect("spawn cellgov_firmware install --force");
    assert!(
        result.status.success(),
        "install --force failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    assert!(
        output.join("sys").join("external").is_dir(),
        "expected sys/external after --force install",
    );
    let _ = std::fs::remove_dir_all(&output);
}
