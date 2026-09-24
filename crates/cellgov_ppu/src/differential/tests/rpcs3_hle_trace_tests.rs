//! HLE trace decoding: the header, a record, resynchronisation past
//! garbage and corrupt records, and the write-overlap query and tally.

use super::*;

fn encode_record(buf: &mut Vec<u8>, rec: &HleCallRecord) {
    buf.extend_from_slice(&RECORD_MAGIC.to_le_bytes());
    buf.extend_from_slice(&rec.step.to_le_bytes());
    buf.extend_from_slice(&rec.lr.to_le_bytes());
    buf.extend_from_slice(&rec.thread_id.to_le_bytes());
    buf.extend_from_slice(&rec.depth.to_le_bytes());
    let name_bytes = rec.name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    for a in &rec.args {
        buf.extend_from_slice(&a.to_le_bytes());
    }
    buf.extend_from_slice(&rec.ret.to_le_bytes());
    buf.extend_from_slice(&(rec.writes.len() as u32).to_le_bytes());
    for w in &rec.writes {
        buf.extend_from_slice(&w.addr.to_le_bytes());
        buf.extend_from_slice(&(w.bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(&w.bytes);
    }
}

fn header() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&HEADER_MAGIC.to_le_bytes());
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf
}

fn build_trace(records: &[HleCallRecord]) -> Vec<u8> {
    let mut buf = header();
    for rec in records {
        encode_record(&mut buf, rec);
    }
    buf
}

fn record(name: &str, step: u64, writes: Vec<(u64, Vec<u8>)>) -> HleCallRecord {
    HleCallRecord {
        step,
        lr: 0,
        thread_id: 0,
        depth: 0,
        name: name.to_string(),
        args: [0; 8],
        ret: 0,
        writes: writes
            .into_iter()
            .map(|(addr, bytes)| HleWrite { addr, bytes })
            .collect(),
    }
}

/// Every event, with I/O failures surfaced as a panic.
fn events(bytes: &[u8]) -> Vec<HleTraceEvent> {
    HleTraceReader::new(bytes)
        .unwrap()
        .map(|event| event.unwrap())
        .collect()
}

fn records(bytes: &[u8]) -> Vec<HleCallRecord> {
    events(bytes)
        .into_iter()
        .filter_map(|event| match event {
            HleTraceEvent::Record(rec) => Some(rec),
            _ => None,
        })
        .collect()
}

#[test]
fn a_record_round_trips_every_field() {
    let rec = HleCallRecord {
        step: 0x1234,
        lr: 0x0001_0200,
        thread_id: 0x0100_0001,
        depth: 2,
        name: "cellGcmInit".to_string(),
        args: [1, 2, 3, 4, 5, 6, 7, 8],
        ret: 0x8001_0002,
        writes: vec![HleWrite {
            addr: 0x101e_3cb8,
            bytes: vec![0xde, 0xad, 0xbe, 0xef],
        }],
    };
    assert_eq!(records(&build_trace(std::slice::from_ref(&rec))), vec![rec]);
}

#[test]
fn records_come_back_in_trace_order() {
    let recs = vec![
        record("cellSysmoduleLoadModule", 0x100, Vec::new()),
        record("cellGcmInit", 0x200, vec![(0x101e_3cb8, vec![0xde])]),
    ];
    assert_eq!(records(&build_trace(&recs)), recs);
}

#[test]
fn a_header_with_no_records_yields_nothing() {
    assert!(events(&header()).is_empty());
}

#[test]
fn the_header_is_refused_by_magic_version_and_length() {
    let mut bad_magic = header();
    bad_magic[0] = 0xAB;
    assert!(matches!(
        HleTraceReader::new(&bad_magic[..]),
        Err(HleTraceError::BadHeaderMagic { .. })
    ));
    let mut bad_version = header();
    bad_version[4..8].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    assert!(matches!(
        HleTraceReader::new(&bad_version[..]),
        Err(HleTraceError::BadVersion { got }) if got == FORMAT_VERSION + 1
    ));
    assert!(matches!(
        HleTraceReader::new(&header()[..6]),
        Err(HleTraceError::UnexpectedEof {
            in_field: "trace version"
        })
    ));
}

#[test]
fn garbage_before_a_record_is_skipped_and_counted() {
    let valid = build_trace(&[record("real_call", 0x100, Vec::new())]);
    let mut bytes = valid[..8].to_vec();
    bytes.extend_from_slice(&0xCAFE_BABEu32.to_le_bytes());
    bytes.extend_from_slice(&[0xAB, 0xCD]);
    bytes.extend_from_slice(&valid[8..]);
    let got = events(&bytes);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(got[0], HleTraceEvent::SkippedBytes(6)), "{got:?}");
    assert!(
        matches!(&got[1], HleTraceEvent::Record(rec) if rec.name == "real_call"),
        "{got:?}"
    );
}

#[test]
fn a_corrupt_record_is_dropped_and_the_next_one_read() {
    let mut bytes = header();
    encode_record(&mut bytes, &record("before", 0x100, Vec::new()));
    // A record whose name length exceeds the cap.
    let mut corrupt = Vec::new();
    encode_record(&mut corrupt, &record("x", 0x200, Vec::new()));
    corrupt[4 + 8 + 8 + 4 + 4..4 + 8 + 8 + 4 + 4 + 4].copy_from_slice(&0xFFFF_u32.to_le_bytes());
    bytes.extend_from_slice(&corrupt);
    encode_record(&mut bytes, &record("after", 0x300, Vec::new()));
    let got = events(&bytes);
    let names: Vec<String> = got
        .iter()
        .map(|event| match event {
            HleTraceEvent::Record(rec) => rec.name.clone(),
            HleTraceEvent::SkippedBytes(n) => format!("skip {n}"),
            HleTraceEvent::DroppedRecord(error) => format!("drop {error}"),
        })
        .collect();
    assert_eq!(
        names.first().map(String::as_str),
        Some("before"),
        "{names:?}"
    );
    assert_eq!(
        names.get(1).map(String::as_str),
        Some("drop record name length 65535 exceeds 1 KiB sanity cap"),
        "{names:?}"
    );
    assert_eq!(names.last().map(String::as_str), Some("after"), "{names:?}");
}

#[test]
fn a_trailing_partial_record_magic_ends_the_trace_quietly() {
    let mut bytes = build_trace(&[record("complete_call", 0x100, Vec::new())]);
    bytes.extend_from_slice(&[0x02, 0x00, 0xE6]);
    let got = events(&bytes);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(matches!(&got[0], HleTraceEvent::Record(rec) if rec.name == "complete_call"));
}

#[test]
fn garbage_running_to_the_end_of_the_trace_is_still_counted() {
    let mut bytes = build_trace(&[record("complete_call", 0x100, Vec::new())]);
    // Seven bytes with no magic: the window takes four, then slides
    // three times before the input ends.
    bytes.extend_from_slice(&[0xAA; 7]);
    let got = events(&bytes);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(got[1], HleTraceEvent::SkippedBytes(3)), "{got:?}");
}

#[test]
fn a_record_cut_short_is_dropped_not_fatal() {
    let mut bytes = build_trace(&[record("complete_call", 0x100, Vec::new())]);
    bytes.extend_from_slice(&RECORD_MAGIC.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 4]);
    let got = events(&bytes);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(
        got[1],
        HleTraceEvent::DroppedRecord(HleTraceError::UnexpectedEof { in_field: "step" })
    ));
}

#[test]
fn an_io_failure_is_fatal_and_ends_the_read() {
    struct Failing(Vec<u8>);
    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                return Err(std::io::Error::other("disk gone"));
            }
            let n = buf.len().min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            Ok(n)
        }
    }
    let mut reader = HleTraceReader::new(Failing(header())).unwrap();
    assert!(matches!(reader.next(), Some(Err(HleTraceError::Io(_)))));
    assert!(reader.next().is_none());
}

#[test]
fn writes_into_finds_only_records_writing_to_the_range() {
    let unrelated = record("noop_b", 0x200, vec![(0x4000_0000, vec![0x01])]);
    let target = record(
        "writes_target",
        0x300,
        vec![(0x101e_3cb8, vec![0x11, 0x22, 0x33, 0x44])],
    );
    assert!(!record("noop_a", 0x100, Vec::new()).writes_into(0x101e_3cb8, 1));
    assert!(!unrelated.writes_into(0x101e_3cb8, 1));
    assert!(target.writes_into(0x101e_3cb8, 1));
    // A mid-write byte, and a range ending exactly where the write starts.
    assert!(target.writes_into(0x101e_3cba, 1));
    assert!(!target.writes_into(0x101e_3cb0, 8));
    assert!(target.writes_into(0x101e_3cb0, 9));
    assert!(!target.writes_into(0x101e_3cbc, 4));
}

#[test]
fn the_tally_ranks_by_writes_then_calls() {
    let mut tally = HleWriteTally::default();
    for rec in [
        record("few", 1, vec![(0, vec![0])]),
        record("many", 2, vec![(0, vec![0]), (4, vec![0])]),
        record("few", 3, Vec::new()),
        // Sorts ahead of "few" by name, so only the call count puts it
        // after.
        record("alone", 4, vec![(0, vec![0])]),
    ] {
        tally.add(&rec);
    }
    let rows: Vec<(&str, usize, usize)> = tally
        .ranked()
        .iter()
        .map(|row| (row.name, row.calls, row.writes))
        .collect();
    assert_eq!(rows, vec![("many", 1, 2), ("few", 2, 1), ("alone", 1, 1)]);
}
