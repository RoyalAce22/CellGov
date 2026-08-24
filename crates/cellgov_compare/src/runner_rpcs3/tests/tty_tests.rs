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
    let dir = std::env::temp_dir().join(format!("cellgov_rpcs3_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", dir.display()));
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
        offset: 0,
        size,
        guest_addr: addr,
    }
}

fn tty_region_at(name: &str, offset: u64, size: u64, addr: u64) -> TtyRegion {
    TtyRegion {
        name: name.into(),
        offset,
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
        tty_region_at("status", 0, 4, 0x500000),
        tty_region_at("value", 4, 4, 0x500004),
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
    let dir = std::env::temp_dir().join(format!("cellgov_rpcs3_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", dir.display()));
    let path = dir.join(format!("tty_nomag_{n}.log"));
    std::fs::write(&path, b"just some TTY noise\n").expect("write");
    let result = parse_tty_log(&path, &[tty_region("r", 4, 0)]);
    assert!(matches!(result, Err(Rpcs3Error::TtyMagicNotFound)));
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tty_log_truncated_payload_returns_error() {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cellgov_rpcs3_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", dir.display()));
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

/// The magic can be the last thing a killed run flushed, leaving the
/// be32 length half-written.
#[test]
fn parse_tty_log_truncated_header_returns_error() {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cellgov_rpcs3_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("scratch dir {} not creatable: {e}", dir.display()));
    let path = dir.join(format!("tty_hdr_{n}.log"));
    let mut log = TTY_MAGIC.to_vec();
    log.extend_from_slice(&[0x00, 0x00]);
    std::fs::write(&path, &log).expect("write");
    match parse_tty_log(&path, &[tty_region("r", 4, 0)]) {
        Err(Rpcs3Error::TtyPayloadTooSmall { expected, actual }) => {
            assert_eq!(expected, 8);
            assert_eq!(actual, 6);
        }
        other => panic!("expected TtyPayloadTooSmall, got {other:?}"),
    }
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

/// A guest emits one struct and names positions inside it, so the
/// regions can sit apart. Summing sizes would slide every region after
/// the gap and hand back neighbouring bytes that parse as plausible.
#[test]
fn regions_are_sliced_at_their_offsets_across_a_gap() {
    let mut payload = vec![0u8; 144];
    payload[..8].copy_from_slice(&[0xAA; 8]);
    payload[16..144].copy_from_slice(&[0xBB; 128]);
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![
        tty_region_at("header", 0, 8, 0),
        tty_region_at("data", 16, 128, 16),
    ];
    let parsed = parse_tty_log(&path, &regions).expect("parse");
    std::fs::remove_file(&path).ok();
    assert_eq!(parsed[0].data, vec![0xAA; 8]);
    assert_eq!(parsed[1].data, vec![0xBB; 128], "the gap was not skipped");
    assert_eq!(parsed[1].addr, 16);
}

/// A stale log, or a guest that emitted twice, leaves two frames. The
/// first is not necessarily the run's; picking it hands back plausible
/// bytes from the wrong capture.
#[test]
fn a_second_frame_past_the_payload_is_refused_rather_than_resolved_by_position() {
    let payload = vec![0xAAu8; 8];
    let mut second = TTY_MAGIC.to_vec();
    second.extend_from_slice(&8_u32.to_be_bytes());
    second.extend_from_slice(&[0xBBu8; 8]);
    let path = write_tty_log(b"", &payload, &second);
    let err = parse_tty_log(&path, &[tty_region("result", 8, 0)]).expect_err("two frames");
    std::fs::remove_file(&path).ok();
    match err {
        Rpcs3Error::TtyFrameAmbiguous { first, second } => {
            assert_eq!(first, 0);
            assert_eq!(second, 16, "the second frame starts right past the payload");
        }
        other => panic!("expected TtyFrameAmbiguous, got {other:?}"),
    }
}

/// The magic is four arbitrary bytes; a region can legitimately hold
/// them. Only an occurrence past the payload is a second frame.
#[test]
fn magic_bytes_inside_the_payload_are_region_data_not_a_second_frame() {
    let mut payload = vec![0u8; 12];
    payload[4..8].copy_from_slice(TTY_MAGIC.as_slice());
    let path = write_tty_log(b"", &payload, b"");
    let parsed = parse_tty_log(&path, &[tty_region("result", 12, 0)]).expect("parse");
    std::fs::remove_file(&path).ok();
    assert_eq!(parsed[0].data, payload);
}

/// The overflowing offset is the number the operator has to correct,
/// so the refusal has to name it and not only the size.
#[test]
fn an_offset_that_overflows_is_named_alongside_its_size() {
    let path = write_tty_log(b"", &[0u8; 8], b"");
    let regions = vec![tty_region_at("wrap", u64::MAX, 1, 0)];
    let err = parse_tty_log(&path, &regions).expect_err("offset + size wraps");
    std::fs::remove_file(&path).ok();
    match err {
        Rpcs3Error::TtyOffsetOverflow {
            ref region_name,
            offset,
            size,
        } => {
            assert_eq!(region_name, "wrap");
            assert_eq!(offset, u64::MAX);
            assert_eq!(size, 1);
        }
        other => panic!("expected TtyOffsetOverflow, got {other:?}"),
    }
    assert!(
        err.to_string().contains(&format!("offset={}", u64::MAX)),
        "the message names the offset: {err}"
    );
}

#[test]
fn a_region_running_past_the_payload_is_rejected() {
    let payload = vec![0u8; 16];
    let path = write_tty_log(b"", &payload, b"");
    let regions = vec![tty_region_at("tail", 12, 8, 12)];
    let err = parse_tty_log(&path, &regions).expect_err("past the end");
    std::fs::remove_file(&path).ok();
    assert!(
        matches!(
            err,
            Rpcs3Error::TtyPayloadTooSmall {
                expected: 20,
                actual: 16
            }
        ),
        "got {err:?}"
    );
}
