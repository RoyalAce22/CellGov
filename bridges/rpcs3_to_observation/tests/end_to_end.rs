//! End-to-end tests: adapter output feeds `cellgov diff observations`.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use cellgov_compare::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedHashes, ObservedOutcome,
};
use cellgov_testkit::scratch::{scratch_labeled, ScratchDir};
use cellgov_trace::StateHash;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> ScratchDir {
    scratch_labeled(name)
}

fn adapter_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rpcs3_to_observation"))
}

fn expected_config_hash_hex() -> String {
    let out = Command::new(adapter_bin())
        .arg("--print-expected-config-hash")
        .output()
        .expect("adapter runs");
    assert!(
        out.status.success(),
        "adapter --print-expected-config-hash failed"
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Locates the sibling `cellgov` binary; `CARGO_BIN_EXE_<name>` only
/// covers binaries in the same package.
fn cellgov_bin() -> PathBuf {
    let me = PathBuf::from(env!("CARGO_BIN_EXE_rpcs3_to_observation"));
    let target_dir = me.parent().expect("adapter has parent dir");
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    target_dir.join(format!("cellgov{exe_suffix}"))
}

#[test]
fn cellgov_and_rpcs3_json_compare_as_match_on_identical_regions() {
    let work = tmp("match");

    let dump_bytes: Vec<u8> = (0..16u8).collect();
    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, &dump_bytes).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x8"

[[regions]]
name = "data"
addr = "0x20000"
size = "0x8"
"#,
    )
    .unwrap();

    let rpcs3_obs_path = work.join("rpcs3_llvm.json");
    let cfg_hash = expected_config_hash_hex();
    let status = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            rpcs3_obs_path.to_str().unwrap(),
            "--config-hash",
            &cfg_hash,
        ])
        .status()
        .expect("adapter runs");
    assert!(status.success(), "adapter exited non-zero");

    let cellgov_obs = Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![
            NamedMemoryRegion {
                name: "code".into(),
                addr: 0x10000,
                data: dump_bytes[0..8].to_vec(),
            },
            NamedMemoryRegion {
                name: "data".into(),
                addr: 0x20000,
                data: dump_bytes[8..16].to_vec(),
            },
        ],
        events: vec![],
        state_hashes: Some(ObservedHashes {
            memory: StateHash::new(0xdead_beef_0000_0001),
            unit_status: StateHash::new(0xdead_beef_0000_0002),
            sync: StateHash::new(0xdead_beef_0000_0003),
        }),
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(1234),
        },
        tty_log: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware: None,
    };
    let cellgov_obs_path = work.join("cellgov.json");
    fs::write(
        &cellgov_obs_path,
        serde_json::to_string_pretty(&cellgov_obs).unwrap(),
    )
    .unwrap();

    let out = Command::new(cellgov_bin())
        .args([
            "diff",
            "observations",
            cellgov_obs_path.to_str().unwrap(),
            rpcs3_obs_path.to_str().unwrap(),
        ])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "diff observations exited non-zero. stdout={stdout} stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("MATCH"),
        "expected MATCH in output, got: {stdout}"
    );
}

#[test]
fn asymmetric_regions_report_diverge_not_schema_error() {
    let work = tmp("diverge");

    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8, 1, 2, 9]).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "r"
addr = "0x10000"
size = "0x4"
"#,
    )
    .unwrap();

    let rpcs3_obs_path = work.join("rpcs3_llvm.json");
    let cfg_hash = expected_config_hash_hex();
    let out = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            rpcs3_obs_path.to_str().unwrap(),
            "--config-hash",
            &cfg_hash,
        ])
        .output()
        .expect("adapter runs");
    assert!(
        out.status.success(),
        "adapter exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let cellgov_obs = Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![NamedMemoryRegion {
            name: "r".into(),
            addr: 0x10000,
            data: vec![0, 1, 2, 3],
        }],
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(1),
        },
        tty_log: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware: None,
    };
    let cellgov_obs_path = work.join("cellgov.json");
    fs::write(
        &cellgov_obs_path,
        serde_json::to_string_pretty(&cellgov_obs).unwrap(),
    )
    .unwrap();

    let out = Command::new(cellgov_bin())
        .args([
            "diff",
            "observations",
            cellgov_obs_path.to_str().unwrap(),
            rpcs3_obs_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("DIVERGE"),
        "expected DIVERGE, got: {stdout}"
    );
    assert!(
        !stdout.contains("parse"),
        "expected real divergence, not a parse/schema error: {stdout}"
    );
}

#[test]
fn adapter_rejects_dump_with_wrong_oracle_config_hash() {
    let work = tmp("bad_config");

    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8; 16]).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "r"
addr = "0x10000"
size = "0x10"
"#,
    )
    .unwrap();

    let out_path = work.join("rpcs3.json");
    let out = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            out_path.to_str().unwrap(),
            "--config-hash",
            "0xdeadbeefdeadbeef",
        ])
        .output()
        .expect("adapter runs");
    assert!(
        !out.status.success(),
        "adapter must fail on config-hash mismatch"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("reference-mode config mismatch"),
        "diagnostic names the contract: {stderr}"
    );
    assert!(
        stderr.contains("0xdeadbeefdeadbeef"),
        "diagnostic echoes the supplied hash: {stderr}"
    );
    assert!(
        !out_path.exists(),
        "adapter must not emit an observation on mismatch"
    );
}

/// The fixture flow in every cell's `REPRODUCTION.md` writes
/// `tests/fixtures/<id>/rpcs3/observation.json`, a fixed name `cellgov
/// dev fixture-gen --rpcs3` reads back. The decoder rule must not
/// forbid it.
#[test]
fn adapter_accepts_the_fixture_tree_output_name() {
    let work = tmp("fixture_name");

    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8; 16]).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x10"
"#,
    )
    .unwrap();

    let out_path = work.join("observation.json");
    let out = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            out_path.to_str().unwrap(),
            "--config-hash",
            &expected_config_hash_hex(),
        ])
        .output()
        .expect("adapter runs");
    assert!(
        out.status.success(),
        "adapter rejected the fixture-tree name: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let obs: Observation = serde_json::from_str(&fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(obs.metadata.runner, "rpcs3-llvm");
}

#[test]
fn adapter_rejects_an_output_named_for_the_other_decoder() {
    let work = tmp("wrong_decoder_name");

    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8; 16]).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x10"
"#,
    )
    .unwrap();

    let out_path = work.join("rpcs3_llvm.json");
    let out = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "interpreter",
            "--output",
            out_path.to_str().unwrap(),
            "--config-hash",
            &expected_config_hash_hex(),
        ])
        .output()
        .expect("adapter runs");
    assert!(!out.status.success(), "adapter must refuse the wrong name");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("interpreter") && stderr.contains("llvm"),
        "diagnostic names both sides: {stderr}"
    );
    assert!(!out_path.exists(), "no observation written on refusal");
}

/// Lay out a runner installation whose default `dev_flash` carries
/// `version.txt`.
fn runner_install(work: &ScratchDir, version_txt: Option<&str>) -> PathBuf {
    let root = work.join("runner");
    if let Some(text) = version_txt {
        let mut path = root.join("dev_flash");
        for c in cellgov_ps3_abi::format::dev_flash::VERSION_TXT_COMPONENTS {
            path.push(c);
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
    } else {
        fs::create_dir_all(&root).unwrap();
    }
    root
}

fn convert_with_runner_dir(work: &ScratchDir, runner_dir: &Path) -> std::process::Output {
    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8; 16]).unwrap();

    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x10"
"#,
    )
    .unwrap();
    let out_path = work.join("rpcs3_llvm.json");

    Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            out_path.to_str().unwrap(),
            "--config-hash",
            &expected_config_hash_hex(),
            "--rpcs3-dir",
            runner_dir.to_str().unwrap(),
        ])
        .output()
        .expect("adapter runs")
}

#[test]
fn the_runner_dir_stamps_the_observation_with_the_version_it_found() {
    let work = tmp("runner_firmware_stamp");
    let runner = runner_install(&work, Some("release:04.9300:\nbuild:1,2:host\n"));
    let out = convert_with_runner_dir(&work, &runner);
    assert!(
        out.status.success(),
        "adapter refused a stamped conversion: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let obs: Observation =
        serde_json::from_str(&fs::read_to_string(work.join("rpcs3_llvm.json")).unwrap()).unwrap();
    assert_eq!(obs.runner_firmware.as_deref(), Some("4.93"));
}

#[test]
fn a_runner_dir_with_no_firmware_refuses_instead_of_writing_an_unstamped_observation() {
    let work = tmp("runner_firmware_missing");
    let runner = runner_install(&work, None);
    let out = convert_with_runner_dir(&work, &runner);
    assert!(!out.status.success(), "adapter must refuse");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--rpcs3-dir"), "{stderr}");
    assert!(
        !work.join("rpcs3_llvm.json").exists(),
        "no observation written on refusal"
    );
}

/// The refusal for a title capture lives in `cellgov dev fixture-gen`,
/// which reads the field back.
#[test]
fn omitting_the_runner_dir_leaves_the_firmware_field_absent() {
    let work = tmp("runner_firmware_absent");
    let dump_path = work.join("rpcs3.dump");
    fs::write(&dump_path, [0u8; 16]).unwrap();
    let manifest_path = work.join("manifest.toml");
    fs::write(
        &manifest_path,
        r#"
[[regions]]
name = "code"
addr = "0x10000"
size = "0x10"
"#,
    )
    .unwrap();
    let out_path = work.join("rpcs3_llvm.json");
    let out = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--decoder",
            "llvm",
            "--output",
            out_path.to_str().unwrap(),
            "--config-hash",
            &expected_config_hash_hex(),
        ])
        .output()
        .expect("adapter runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let obs: Observation = serde_json::from_str(&fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(obs.runner_firmware, None);
}

#[test]
fn adapter_prints_expected_config_hash() {
    let out = Command::new(adapter_bin())
        .arg("--print-expected-config-hash")
        .output()
        .expect("adapter runs");
    assert!(out.status.success(), "expected-hash command ran clean");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    assert!(
        trimmed.starts_with("0x"),
        "hash output is hex-formatted: {trimmed}"
    );
    assert_eq!(trimmed.len(), 18, "0x + 16 hex digits: {trimmed}");
}
