//! RPCS3 call-trace parsing, resync past garbage, and write-address filtering.

use super::*;
use std::io::Write;

fn build_trace(records: &[CallRecord]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.write_all(&HEADER_MAGIC.to_le_bytes()).unwrap();
    buf.write_all(&TRACE_VERSION.to_le_bytes()).unwrap();
    for rec in records {
        buf.write_all(&RECORD_MAGIC.to_le_bytes()).unwrap();
        buf.write_all(&rec.step.to_le_bytes()).unwrap();
        buf.write_all(&rec.lr.to_le_bytes()).unwrap();
        buf.write_all(&rec.thread_id.to_le_bytes()).unwrap();
        buf.write_all(&rec.depth.to_le_bytes()).unwrap();
        let name_bytes = rec.name.as_bytes();
        buf.write_all(&(name_bytes.len() as u32).to_le_bytes())
            .unwrap();
        buf.write_all(name_bytes).unwrap();
        for a in &rec.args {
            buf.write_all(&a.to_le_bytes()).unwrap();
        }
        buf.write_all(&rec.ret.to_le_bytes()).unwrap();
        buf.write_all(&(rec.writes.len() as u32).to_le_bytes())
            .unwrap();
        for w in &rec.writes {
            buf.write_all(&w.addr.to_le_bytes()).unwrap();
            buf.write_all(&(w.bytes.len() as u32).to_le_bytes())
                .unwrap();
            buf.write_all(&w.bytes).unwrap();
        }
    }
    buf
}

fn write_temp(bytes: &[u8], label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("cellgov_htrc_{label}_{pid}_{n}.bin"));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn fixture_record(name: &str, step: u64, writes: Vec<(u64, Vec<u8>)>) -> CallRecord {
    CallRecord {
        step,
        lr: 0,
        thread_id: 0,
        depth: 0,
        name: name.to_string(),
        args: [0; 8],
        ret: 0,
        writes: writes
            .into_iter()
            .map(|(addr, bytes)| WriteEntry { addr, bytes })
            .collect(),
    }
}

#[test]
fn parse_round_trips_a_minimal_trace() {
    let records = vec![
        fixture_record("cellSysmoduleLoadModule", 0x100, Vec::new()),
        fixture_record(
            "cellGcmInit",
            0x200,
            vec![(0x101e3cb8, vec![0xde, 0xad, 0xbe, 0xef])],
        ),
    ];
    let bytes = build_trace(&records);
    let path = write_temp(&bytes, "round_trip");
    let parsed = parse(&path).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "cellSysmoduleLoadModule");
    assert_eq!(parsed[1].name, "cellGcmInit");
    assert_eq!(parsed[1].writes.len(), 1);
    assert_eq!(parsed[1].writes[0].addr, 0x101e3cb8);
    assert_eq!(parsed[1].writes[0].bytes, vec![0xde, 0xad, 0xbe, 0xef]);
    std::fs::remove_file(&path).ok();
}

#[test]
fn filter_addr_finds_only_records_writing_to_query_range() {
    let records = vec![
        fixture_record("noop_a", 0x100, Vec::new()),
        fixture_record(
            "noop_b",
            0x200,
            vec![(0x40000000, vec![0x01])], // unrelated write
        ),
        fixture_record(
            "writes_target",
            0x300,
            vec![(0x101e3cb8, vec![0x11, 0x22, 0x33, 0x44])],
        ),
    ];
    let hits = filter_addr_range(&records, 0x101e3cb8, 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "writes_target");
}

#[test]
fn filter_addr_range_preserves_input_order() {
    let records = vec![
        fixture_record(
            "first_writer",
            0x300, // higher PC than second_writer's PC
            vec![(0x101e3cb8, vec![0x11, 0x22, 0x33, 0x44])],
        ),
        fixture_record(
            "second_writer",
            0x100, // lower PC, still emitted second in time
            vec![(0x101e3cb8, vec![0xff, 0xff, 0xff, 0xff])],
        ),
    ];
    let hits = filter_addr_range(&records, 0x101e3cb8, 4);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].name, "first_writer");
    assert_eq!(hits[1].name, "second_writer");
}

#[test]
fn filter_addr_range_handles_partial_overlap() {
    let records = vec![fixture_record(
        "partial_overlap",
        0x100,
        vec![(0x101e3cb8, vec![0x11, 0x22, 0x33, 0x44])],
    )];
    let hits = filter_addr_range(&records, 0x101e3cba, 1); // mid-write byte
    assert_eq!(hits.len(), 1);
}

#[test]
fn parse_rejects_bad_header_magic() {
    let mut bytes = vec![0u8; 8];
    bytes[0] = 0xAB; // wrong magic
    let path = write_temp(&bytes, "bad_magic");
    let err = parse(&path).unwrap_err();
    match err {
        ParseError::BadHeaderMagic { .. } => {}
        other => panic!("expected BadHeaderMagic, got {other:?}"),
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_resyncs_past_garbage_to_find_valid_records() {
    let records = vec![fixture_record("real_call", 0x100, Vec::new())];
    let valid = build_trace(&records);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&valid[..8]); // header
    bytes.extend_from_slice(&0xCAFEBABEu32.to_le_bytes()); // garbage
    bytes.extend_from_slice(&[0xAB, 0xCD]); // more garbage
    bytes.extend_from_slice(&valid[8..]); // valid record body
    let path = write_temp(&bytes, "resync_garbage");
    let parsed = parse(&path).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "real_call");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tolerates_trailing_partial_record_magic() {
    let records = vec![fixture_record("complete_call", 0x100, Vec::new())];
    let mut bytes = build_trace(&records);
    bytes.push(0x02);
    bytes.push(0xC0);
    bytes.push(0xE6);
    // Missing the 4th byte of the magic -> partial.
    let path = write_temp(&bytes, "partial_magic");
    let parsed = parse(&path).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "complete_call");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_tolerates_truncation_inside_a_record_body() {
    let records = vec![fixture_record("complete_call", 0x100, Vec::new())];
    let mut bytes = build_trace(&records);
    bytes.extend_from_slice(&RECORD_MAGIC.to_le_bytes());
    // Half a step (4 of 8 bytes); reader hits EOF.
    bytes.extend_from_slice(&[0u8; 4]);
    let path = write_temp(&bytes, "partial_body");
    let parsed = parse(&path).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "complete_call");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_handles_empty_trace_with_only_header() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&HEADER_MAGIC.to_le_bytes());
    bytes.extend_from_slice(&TRACE_VERSION.to_le_bytes());
    let path = write_temp(&bytes, "empty");
    let parsed = parse(&path).unwrap();
    assert!(parsed.is_empty());
    std::fs::remove_file(&path).ok();
}

/// Parse the entire trace file into a `Vec<CallRecord>`. Production
/// callers stream via [`parse_streaming`] for bounded memory.
fn parse(path: &Path) -> Result<Vec<CallRecord>, ParseError> {
    let mut records = Vec::new();
    parse_streaming(path, |rec| {
        records.push(rec);
        Ok(())
    })?;
    Ok(records)
}

/// Records whose write set covers any byte of `[addr, addr+len)`,
/// returned in input order.
fn filter_addr_range(records: &[CallRecord], addr: u64, len: u64) -> Vec<&CallRecord> {
    let end = addr.saturating_add(len);
    records
        .iter()
        .filter(|rec| {
            rec.writes.iter().any(|w| {
                let w_end = w.addr.saturating_add(w.bytes.len() as u64);
                w.addr < end && addr < w_end
            })
        })
        .collect()
}
