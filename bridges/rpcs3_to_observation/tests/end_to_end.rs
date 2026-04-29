//! End-to-end tests: adapter output feeds `cellgov_cli compare-observations`.

use cellgov_compare::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedHashes, ObservedOutcome,
};
use cellgov_trace::StateHash;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cellgov_9a4_{name}"));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn adapter_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rpcs3_to_observation"))
}

/// Returns the adapter's oracle-mode config hash as a "0x..." string.
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

/// Locates the sibling `cellgov_cli` binary; `CARGO_BIN_EXE_<name>` only
/// covers binaries in the same package.
fn cellgov_cli_bin() -> PathBuf {
    let me = PathBuf::from(env!("CARGO_BIN_EXE_rpcs3_to_observation"));
    let target_dir = me.parent().expect("adapter has parent dir");
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    target_dir.join(format!("cellgov_cli{exe_suffix}"))
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

    let rpcs3_obs_path = work.join("rpcs3.json");
    let cfg_hash = expected_config_hash_hex();
    let status = Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
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
    };
    let cellgov_obs_path = work.join("cellgov.json");
    fs::write(
        &cellgov_obs_path,
        serde_json::to_string_pretty(&cellgov_obs).unwrap(),
    )
    .unwrap();

    let out = Command::new(cellgov_cli_bin())
        .args([
            "compare-observations",
            cellgov_obs_path.to_str().unwrap(),
            rpcs3_obs_path.to_str().unwrap(),
        ])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "compare-observations exited non-zero. stdout={stdout} stderr={}",
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

    let rpcs3_obs_path = work.join("rpcs3.json");
    let cfg_hash = expected_config_hash_hex();
    Command::new(adapter_bin())
        .args([
            "--dump",
            dump_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--outcome",
            "completed",
            "--output",
            rpcs3_obs_path.to_str().unwrap(),
            "--config-hash",
            &cfg_hash,
        ])
        .status()
        .unwrap();

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
    };
    let cellgov_obs_path = work.join("cellgov.json");
    fs::write(
        &cellgov_obs_path,
        serde_json::to_string_pretty(&cellgov_obs).unwrap(),
    )
    .unwrap();

    let out = Command::new(cellgov_cli_bin())
        .args([
            "compare-observations",
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
        stderr.contains("oracle-mode config mismatch"),
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
