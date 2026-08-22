//! Integration tests for `cellgov_install install` against a real
//! PS3UPDAT.PUP.
//!
//! `--output` names the VFS root, so the firmware lands in its
//! `dev_flash` mount beside whatever `install-game` put in
//! `dev_hdd0` / `dev_bdvd`.
//!
//! The PUP is operator-owned and read from the dump root
//! (`common/dumps.rs`); nothing in the repo ships one. This suite is
//! compiled only under the `firmware-dumps` feature, which declares
//! that fixture present: a missing one is a hard failure rather than
//! a skip.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::PathBuf;
use std::process::Command;

#[path = "common/dumps.rs"]
mod dumps;
#[path = "common/scratch.rs"]
mod scratch;

/// Mount the firmware lands in under the VFS root `--output` names.
const DEV_FLASH: &str = "dev_flash";

/// The PUP under test.
///
/// # Panics
///
/// If the dump root holds no PUP.
fn locate_pup() -> PathBuf {
    let p = dumps::ps3updat_pup();
    assert!(
        p.is_file(),
        "firmware-dumps: no PUP at {}. Either drop a PS3UPDAT.PUP \
         there or point {} at a dump root holding one.",
        p.display(),
        dumps::ENV_DUMPS_DIR,
    );
    p
}

fn run_install(pup: &PathBuf, vfs_root: &std::path::Path, force: bool) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_cellgov_install");
    let mut cmd = Command::new(bin);
    cmd.arg("install")
        .arg(pup)
        .arg("--output")
        .arg(vfs_root.as_os_str());
    if force {
        cmd.arg("--force");
    }
    cmd.output().expect("spawn cellgov_install install")
}

#[test]
fn install_with_an_empty_vfs_populates_dev_flash_sys_external() {
    let pup = locate_pup();
    let output = scratch::ScratchDir::new("fw_happy");
    let result = run_install(&pup, &output, false);
    assert!(
        result.status.success(),
        "install failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );

    let sys_external = output.join(DEV_FLASH).join("sys").join("external");
    assert!(
        sys_external.is_dir(),
        "expected {} after install",
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
    // The manifest describes the firmware image, so it sits in the
    // mount it covers rather than at the VFS root.
    assert!(
        output.join(DEV_FLASH).join("firmware.toml").is_file(),
        "expected firmware.toml inside the dev_flash mount"
    );
}

#[test]
fn install_refuses_a_non_empty_dev_flash_without_force() {
    let pup = locate_pup();
    let output = scratch::ScratchDir::new("fw_refuse");
    // The preflight is scoped to the mount the install writes, so the
    // decoy has to live there -- a file at the VFS root is legitimate
    // (dev_hdd0 and friends) and must not block a firmware install.
    let dev_flash = output.join(DEV_FLASH);
    std::fs::create_dir_all(&dev_flash).unwrap();
    std::fs::write(dev_flash.join("decoy.txt"), b"existing").unwrap();

    let result = run_install(&pup, &output, false);
    assert!(
        !result.status.success(),
        "expected install to refuse a non-empty dev_flash without --force\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("exists and is non-empty"),
        "expected the refuse-overwrite message in stderr, got:\n{stderr}",
    );
}

#[test]
fn a_populated_vfs_root_does_not_block_a_firmware_install() {
    let pup = locate_pup();
    let output = scratch::ScratchDir::new("fw_sibling");
    std::fs::create_dir_all(output.join("dev_hdd0/game/NPUA80001")).unwrap();
    std::fs::write(output.join("dev_hdd0/game/NPUA80001/x.bin"), b"game").unwrap();

    let result = run_install(&pup, &output, false);
    assert!(
        result.status.success(),
        "a populated dev_hdd0 must not block a firmware install.\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    assert!(
        output.join("dev_hdd0/game/NPUA80001/x.bin").is_file(),
        "the firmware install must not disturb a sibling mount"
    );
}

#[test]
fn force_allows_a_non_empty_dev_flash() {
    let pup = locate_pup();
    let output = scratch::ScratchDir::new("fw_force");
    let dev_flash = output.join(DEV_FLASH);
    std::fs::create_dir_all(&dev_flash).unwrap();
    std::fs::write(dev_flash.join("decoy.txt"), b"existing").unwrap();

    let result = run_install(&pup, &output, true);
    assert!(
        result.status.success(),
        "install --force failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    assert!(
        dev_flash.join("sys").join("external").is_dir(),
        "expected dev_flash/sys/external after --force install",
    );
}
