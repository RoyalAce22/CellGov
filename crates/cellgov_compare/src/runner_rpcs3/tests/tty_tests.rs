//! Framed TTY-log parsing: magic search amid noise, payload bounds, and observation assembly.

use super::*;
use crate::observation::ObservedOutcome;
use crate::runner_rpcs3::config::Rpcs3Decoder;
use crate::runner_rpcs3::observe_from_tty;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Build a TTY log file with the framed protocol: CGOV + len + payload.
fn write_tty_log(prefix: &[u8], payload: &[u8], suffix: &[u8]) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("tty_{n}.log"));
    let mut f = std::fs::File::create(&path).expect("create tty");
    f.write_all(prefix).expect("write prefix");
    f.write_all(TTY_MAGIC).expect("write magic");
    f.write_all(&(payload.len() as u32).to_be_bytes())
        .expect("write len");
    f.write_all(payload).expect("write payload");
    f.write_all(suffix).expect("write suffix");
    path
}

fn tty_region(name: &str, size: u64, addr: u64) -> TtyRegion {
    TtyRegion {
        name: name.into(),
        size,
        guest_addr: addr,
    }
}

#[test]
fn parse_tty_log_extracts_single_region() {
    let payload = vec![0x00, 0x00, 0x00, 0x01, 0x13, 0x37, 0xBA, 0xAD];
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![tty_region("result", 8, 0x500000)];
    let parsed = parse_tty_log(&path, &regions).expect("parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "result");
    assert_eq!(parsed[0].addr, 0x500000);
    assert_eq!(parsed[0].data, payload);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_extracts_multiple_regions() {
    let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![
        tty_region("status", 4, 0x500000),
        tty_region("value", 4, 0x500004),
    ];
    let parsed = parse_tty_log(&path, &regions).expect("parse");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(parsed[1].data, vec![0x11, 0x22, 0x33, 0x44]);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_finds_tag_after_noise() {
    let noise = b"SPU Thread Group [0x1] started\nTest running...\n";
    let payload = vec![0x42, 0x00, 0x00, 0x00];
    let path = write_tty_log(noise, &payload, b"\nDone.\n");
    let regions = vec![tty_region("result", 4, 0x10000)];
    let parsed = parse_tty_log(&path, &regions).expect("parse");
    assert_eq!(parsed[0].data, vec![0x42, 0x00, 0x00, 0x00]);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_no_magic_returns_error() {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("tty_nomag_{n}.log"));
    std::fs::write(&path, b"just some TTY noise\n").expect("write");
    let result = parse_tty_log(&path, &[tty_region("r", 4, 0)]);
    assert!(matches!(result, Err(Rpcs3Error::TtyMagicNotFound)));
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_truncated_payload_returns_error() {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("tty_trunc_{n}.log"));
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(TTY_MAGIC).expect("magic");
    f.write_all(&100_u32.to_be_bytes()).expect("len");
    f.write_all(&[0u8; 10]).expect("short payload");
    drop(f);
    let result = parse_tty_log(&path, &[tty_region("r", 8, 0)]);
    assert!(matches!(result, Err(Rpcs3Error::TtyPayloadTooSmall { .. })));
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_regions_exceed_payload_returns_error() {
    let payload = vec![0u8; 4];
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![tty_region("big", 8, 0)];
    let result = parse_tty_log(&path, &regions);
    assert!(matches!(result, Err(Rpcs3Error::TtyPayloadTooSmall { .. })));
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_empty_regions_returns_empty_vec() {
    let payload = vec![0u8; 8];
    let path = write_tty_log(b"", &payload, b"");
    let parsed = parse_tty_log(&path, &[]).expect("parse");
    assert!(parsed.is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_nonexistent_file_returns_error() {
    let result = parse_tty_log(Path::new("/nonexistent/tty.log"), &[]);
    assert!(matches!(result, Err(Rpcs3Error::TtyRead(_))));
}

#[test]
fn observe_from_tty_builds_observation() {
    let payload = vec![0x00, 0x00, 0x00, 0x00, 0x13, 0x37, 0xBA, 0xAD];
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![tty_region("result", 8, 0)];
    let obs = observe_from_tty(&path, &regions, Rpcs3Decoder::Interpreter).expect("observe");
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
    assert_eq!(obs.memory_regions.len(), 1);
    assert_eq!(obs.memory_regions[0].data, payload);
    assert_eq!(obs.metadata.runner, "rpcs3-interpreter");
    assert!(obs.state_hashes.is_none());
    std::fs::remove_file(&path).ok();
}

#[test]
fn observe_from_tty_with_real_baseline() {
    let tty_path = Path::new("../../baselines/spu_fixed_value/rpcs3_interpreter.tty");
    if !tty_path.exists() {
        return;
    }
    let regions = vec![tty_region("result", 8, 0)];
    let obs = observe_from_tty(tty_path, &regions, Rpcs3Decoder::Interpreter).expect("observe");
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
    assert_eq!(obs.memory_regions[0].data[4..8], [0x13, 0x37, 0xBA, 0xAD]);
}
