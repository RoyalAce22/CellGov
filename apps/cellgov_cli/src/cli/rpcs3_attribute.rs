//! `rpcs3-attribute` subcommand: read an RPCS3 HLE trace produced by
//! the patched build (`bridges/rpcs3-patch/0002-cellgov-hle-trace.patch`)
//! and answer "which HLE call wrote this guest address?"
//!
//! Trace format pinned in `tools/rpcs3-src/rpcs3/Emu/Cell/cellgov_hle_trace.h`.
//! Records are emitted at every BIND_FUNC entry+exit pair; each record
//! lists the writes the call made to any `CELLGOV_HLE_WATCH` region
//! (diff against the entry-time snapshot). Nested calls emit at every
//! level so the deepest-touching frame is the precise attribution.
//!
//! Typical investigation flow:
//!
//! ```text
//! CELLGOV_HLE_TRACE_PATH=flow.htrc \
//! CELLGOV_HLE_WATCH=0x101e3cb8:8 \
//! tools/rpcs3-src/build-msvc/bin/rpcs3.exe --headless flow.elf
//!
//! cellgov_cli rpcs3-attribute --trace flow.htrc --addr 0x101e3cb8
//! ```
//!
//! That replaces an open-ended Ghidra hunt with a single deterministic
//! answer per fault address.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use super::args::{find_flag_value, parse_hex_u64};
use super::exit::die;

const HEADER_MAGIC: u32 = 0xC0E6_0001;
const RECORD_MAGIC: u32 = 0xC0E6_0002;
const TRACE_VERSION: u32 = 2;

/// One emitted HLE-call record. Mirrors the binary record on disk.
///
/// `lr` is the PPU LR at HLE entry. For HLE module functions this is
/// the user-code call site (a real PC in the title binary). For
/// syscalls it is the syscall-stub return PC. For synthetic
/// `<guest_code>` drift records it is the LR captured at the prior
/// HLE call's exit (= the user-code site running between the two
/// calls).
#[derive(Debug, Clone)]
pub struct CallRecord {
    pub step: u64,
    pub lr: u64,
    pub thread_id: u32,
    pub depth: u32,
    pub name: String,
    pub args: [u64; 8],
    pub ret: u64,
    pub writes: Vec<WriteEntry>,
}

#[derive(Debug, Clone)]
pub struct WriteEntry {
    pub addr: u64,
    pub bytes: Vec<u8>,
}

/// Parser error variants. Local enum, no `From` impls; the dispatch
/// site formats and dies.
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    BadHeaderMagic { got: u32 },
    BadVersion { got: u32 },
    NameTooLong { len: u32 },
    WriteTooLarge { size: u32 },
    UnexpectedEof { in_field: &'static str },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::BadHeaderMagic { got } => {
                write!(
                    f,
                    "trace header magic mismatch: got 0x{got:08x}, expected 0x{HEADER_MAGIC:08x}"
                )
            }
            Self::BadVersion { got } => {
                write!(
                    f,
                    "trace version {got} unsupported (this build expects {TRACE_VERSION})"
                )
            }
            Self::NameTooLong { len } => {
                write!(f, "record name length {len} exceeds 1 KiB sanity cap")
            }
            Self::WriteTooLarge { size } => {
                write!(f, "write payload size {size} exceeds 1 MiB sanity cap")
            }
            Self::UnexpectedEof { in_field } => {
                write!(f, "unexpected EOF while reading {in_field}")
            }
        }
    }
}

/// Parse the entire trace file into a `Vec<CallRecord>`. Only used
/// by tests; production callers stream via [`parse_streaming`] so
/// multi-GB traces stay in bounded memory.
#[cfg(test)]
pub fn parse(path: &Path) -> Result<Vec<CallRecord>, ParseError> {
    // Convenience wrapper around `parse_streaming` for tests and
    // small traces. Callers that handle multi-GB files (the common
    // production case for long boots) should use
    // `parse_streaming` directly so they can filter on the fly
    // without buffering every record.
    let mut records = Vec::new();
    parse_streaming(path, |rec| {
        records.push(rec);
        Ok(())
    })?;
    Ok(records)
}

/// Stream every record in the trace through `on_record`. Bounded
/// memory regardless of trace size. The callback returns `Err` to
/// abort early.
pub fn parse_streaming<F>(path: &Path, mut on_record: F) -> Result<(), ParseError>
where
    F: FnMut(CallRecord) -> Result<(), ParseError>,
{
    let file = File::open(path).map_err(ParseError::Io)?;
    let mut reader = BufReader::with_capacity(1 << 20, file); // 1 MiB buf

    let header_magic = read_u32(&mut reader, "header magic")?;
    if header_magic != HEADER_MAGIC {
        return Err(ParseError::BadHeaderMagic { got: header_magic });
    }
    let version = read_u32(&mut reader, "trace version")?;
    if version != TRACE_VERSION {
        return Err(ParseError::BadVersion { got: version });
    }

    let mut resyncs = 0usize;
    // Sliding 4-byte magic window. We read exactly 4 bytes at every
    // attempt (read_exact-style) so a BufReader buffer boundary can
    // never short-circuit us into thinking we hit EOF. On a real
    // EOF the first read() returns 0; only THEN do we exit cleanly.
    let mut window: [u8; 4] = [0; 4];
    let mut have_window = false;
    loop {
        if !have_window {
            // Read up to 4 bytes; loop on partials.
            let mut filled = 0usize;
            while filled < 4 {
                let n = reader.read(&mut window[filled..]).map_err(ParseError::Io)?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            if filled == 0 {
                break; // clean EOF on record boundary
            }
            if filled < 4 {
                break; // genuinely partial trailing bytes
            }
            have_window = true;
        }
        let magic = u32::from_le_bytes(window);
        if magic != RECORD_MAGIC {
            // Slide the window left by one byte and read one fresh.
            window[0] = window[1];
            window[1] = window[2];
            window[2] = window[3];
            let mut one = [0u8; 1];
            match reader.read(&mut one).map_err(ParseError::Io)? {
                0 => break,
                _ => {}
            }
            window[3] = one[0];
            resyncs += 1;
            continue;
        }
        // Magic matched -- consume and parse the body. If the body
        // is malformed (NameTooLong, WriteTooLarge, or stale
        // UnexpectedEof on a fresh stream-while-being-written
        // file), DON'T bail; resync into the next valid magic so
        // the rest of the trace stays usable. The `Io` error path
        // still surfaces (matched on `is_some_io` below).
        have_window = false;
        match parse_one_record(&mut reader) {
            Ok(rec) => on_record(rec)?,
            Err(e) => {
                if matches!(e, ParseError::Io(_)) {
                    return Err(e);
                }
                resyncs += 1;
                // Force the next iteration to re-read a fresh 4-byte
                // window starting at the current stream position.
                continue;
            }
        }
    }
    if resyncs > 0 {
        eprintln!(
            "rpcs3-attribute: skipped {resyncs} byte(s) of corrupted/partial trace data while resyncing",
        );
    }
    Ok(())
}

fn parse_one_record<R: Read>(reader: &mut R) -> Result<CallRecord, ParseError> {
    let step = read_u64(reader, "step")?;
    let lr = read_u64(reader, "lr")?;
    let thread_id = read_u32(reader, "thread_id")?;
    let depth = read_u32(reader, "depth")?;
    let name_len = read_u32(reader, "name_len")?;
    if name_len > 1024 {
        return Err(ParseError::NameTooLong { len: name_len });
    }
    let mut name_bytes = vec![0u8; name_len as usize];
    reader
        .read_exact(&mut name_bytes)
        .map_err(|_| ParseError::UnexpectedEof { in_field: "name" })?;
    let name = String::from_utf8_lossy(&name_bytes).into_owned();

    let mut args = [0u64; 8];
    for slot in &mut args {
        *slot = read_u64(reader, "args[i]")?;
    }
    let ret = read_u64(reader, "ret")?;

    let num_writes = read_u32(reader, "num_writes")?;
    let mut writes = Vec::with_capacity(num_writes as usize);
    for _ in 0..num_writes {
        let addr = read_u64(reader, "write.addr")?;
        let size = read_u32(reader, "write.size")?;
        if size > 1 << 20 {
            return Err(ParseError::WriteTooLarge { size });
        }
        let mut bytes = vec![0u8; size as usize];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| ParseError::UnexpectedEof {
                in_field: "write.bytes",
            })?;
        writes.push(WriteEntry { addr, bytes });
    }

    Ok(CallRecord {
        step,
        lr,
        thread_id,
        depth,
        name,
        args,
        ret,
        writes,
    })
}

fn read_u32<R: Read>(r: &mut R, field: &'static str) -> Result<u32, ParseError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)
        .map_err(|_| ParseError::UnexpectedEof { in_field: field })?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R, field: &'static str) -> Result<u64, ParseError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)
        .map_err(|_| ParseError::UnexpectedEof { in_field: field })?;
    Ok(u64::from_le_bytes(buf))
}

/// Records whose write set covers any byte of `[addr, addr+len)`.
/// Returns matches in input order (callers feed records in
/// chronological emit order; the parser preserves that and so does
/// this filter). The earliest write is therefore the first hit.
#[cfg(test)]
pub fn filter_addr_range(records: &[CallRecord], addr: u64, len: u64) -> Vec<&CallRecord> {
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

/// Top-level entry point. Parses CLI args and runs one of the query
/// modes:
///
///   --addr 0xADDR [--len N]    show calls writing into [addr, addr+len)
///   --list                     dump every record (verbose)
///   --ranked                   group by name, show write counts
///
/// All modes stream the trace; memory cost is bounded by the result
/// set rather than the trace size, so multi-GB traces are fine.
pub fn run(args: &[String]) {
    let trace_path = find_flag_value(args, "--trace")
        .unwrap_or_else(|| die("usage: cellgov_cli rpcs3-attribute --trace <path> [--addr 0xADDR] [--len N] [--list] [--ranked] [--name SUBSTR]"));
    let path = Path::new(&trace_path);
    if !path.is_file() {
        die(&format!(
            "trace file not found: {trace_path} (did the patched RPCS3 produce one?)"
        ));
    }

    let want_list = args.iter().any(|a| a == "--list");
    let want_ranked = args.iter().any(|a| a == "--ranked");
    let addr_arg = find_flag_value(args, "--addr");
    let name_filter = find_flag_value(args, "--name");

    if !want_list && !want_ranked && addr_arg.is_none() && name_filter.is_none() {
        die(
            "rpcs3-attribute: pick a query mode: --addr 0xADDR, --list, --ranked, or --name SUBSTR",
        );
    }

    let addr_filter: Option<(u64, u64)> = addr_arg.map(|addr_s| {
        let addr = parse_hex_u64(&addr_s, "--addr");
        let len = find_flag_value(args, "--len")
            .map(|s| parse_hex_u64(&s, "--len"))
            .unwrap_or(1);
        (addr, len)
    });

    let mut total_records = 0usize;
    let mut hits: Vec<CallRecord> = Vec::new();
    let mut name_hits: Vec<CallRecord> = Vec::new();
    let mut tally: std::collections::BTreeMap<String, (usize, usize)> = Default::default();

    if let Err(e) = parse_streaming(path, |rec| {
        total_records += 1;
        if total_records.is_multiple_of(1_000_000) {
            eprintln!("rpcs3-attribute: streamed {total_records} records...");
        }
        if want_list {
            print_record(&rec, "");
        }
        if want_ranked {
            let entry = tally.entry(rec.name.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += rec.writes.len();
        }
        if let Some((addr, len)) = addr_filter {
            let end = addr.saturating_add(len);
            if rec.writes.iter().any(|w| {
                let w_end = w.addr.saturating_add(w.bytes.len() as u64);
                w.addr < end && addr < w_end
            }) {
                hits.push(rec.clone());
            }
        }
        if let Some(needle) = name_filter.as_deref() {
            if rec.name.contains(needle) {
                name_hits.push(rec);
            }
        }
        Ok(())
    }) {
        die(&format!("failed to parse trace: {e}"));
    }

    eprintln!("rpcs3-attribute: streamed {total_records} record(s) from {trace_path}",);

    if want_ranked {
        let mut rows: Vec<(&str, usize, usize)> = tally
            .iter()
            .map(|(n, (c, w))| (n.as_str(), *c, *w))
            .collect();
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        println!("{:>8}  {:>8}  name", "writes", "calls");
        for (name, calls, writes) in rows {
            println!("{writes:>8}  {calls:>8}  {name}");
        }
    }

    if let Some((addr, len)) = addr_filter {
        if hits.is_empty() {
            println!(
                "no records wrote to [0x{addr:016x}, 0x{:016x}). Address may be untouched, or the watch list passed to RPCS3 did not include it.",
                addr.saturating_add(len),
            );
        } else {
            // Records arrive from `parse_streaming` in file order, which
            // is the chronological emit order (single-writer mutex
            // around the trace file). DO NOT sort by `step`: that field
            // holds the PPU CIA at HLE entry, which is a code address,
            // not a timestamp.
            println!(
                "{} record(s) wrote into [0x{addr:016x}, 0x{:016x}) in chronological order:",
                hits.len(),
                addr.saturating_add(len),
            );
            for rec in &hits {
                print_record(rec, "  ");
            }
        }
    }

    if let Some(needle) = name_filter.as_deref() {
        if name_hits.is_empty() {
            println!("no records matched name substring {needle:?}");
        } else {
            println!(
                "{} record(s) matched name substring {needle:?} in chronological order:",
                name_hits.len(),
            );
            for rec in &name_hits {
                print_record(rec, "  ");
            }
        }
    }
}

fn print_record(rec: &CallRecord, indent: &str) {
    println!(
        "{indent}step=0x{:016x} lr=0x{:016x} tid={} depth={} {} ret=0x{:x}",
        rec.step, rec.lr, rec.thread_id, rec.depth, rec.name, rec.ret,
    );
    println!(
        "{indent}  args r3..r10 = [{:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}]",
        rec.args[0],
        rec.args[1],
        rec.args[2],
        rec.args[3],
        rec.args[4],
        rec.args[5],
        rec.args[6],
        rec.args[7],
    );
    for w in &rec.writes {
        let hex: String = w
            .bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{indent}  write 0x{:016x} ({} byte{}): {}",
            w.addr,
            w.bytes.len(),
            if w.bytes.len() == 1 { "" } else { "s" },
            hex,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal trace blob in the on-disk format. Used to
    /// pin the parser without needing a real RPCS3 run.
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
        // Records are passed in chronological emit order (file
        // order from `parse_streaming`); the filter must NOT
        // reorder them. `step` is a code address, not a timestamp.
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
        // Watch on a single byte that sits inside a multi-byte write.
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
        // Resync semantics: garbage between the header and the
        // first valid record is skipped (with a stderr warning).
        // A real-world hit would be a buggy partial flush in the
        // RPCS3 hook; the trace stays useful instead of being
        // wholesale rejected.
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
        // Simulates RPCS3 killed mid-fwrite: complete record + 3
        // dangling bytes of the next magic. Parser must keep the
        // complete record and silently truncate.
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
        // Complete record + a record that has its magic but is cut
        // off mid-step. Parser must keep the complete record.
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
}
