//! `rpcs3-attribute` subcommand: read an RPCS3 HLE trace produced by
//! the patched build (`bridges/rpcs3-patch/0002-cellgov-hle-trace.patch`)
//! and answer "which HLE call wrote this guest address?"
//!
//! Trace format pinned in the patch's `cellgov_hle_trace.h` header.
//! Records are emitted at every BIND_FUNC entry+exit pair; each record
//! lists the writes the call made to any `CELLGOV_HLE_WATCH` region
//! (diff against the entry-time snapshot).
//!
//! # Examples
//!
//! ```text
//! CELLGOV_HLE_TRACE_PATH=flow.htrc \
//! CELLGOV_HLE_WATCH=0x101e3cb8:8 \
//! tools/rpcs3-src/build-msvc/bin/rpcs3.exe --headless flow.elf
//!
//! cellgov_cli rpcs3-attribute --trace flow.htrc --addr 0x101e3cb8
//! ```

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

/// Failure modes while parsing the HLE trace.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error(
        "trace header magic mismatch: got 0x{got:08x}, expected 0x{:08x}",
        HEADER_MAGIC
    )]
    BadHeaderMagic { got: u32 },
    #[error(
        "trace version {got} unsupported (this build expects {})",
        TRACE_VERSION
    )]
    BadVersion { got: u32 },
    #[error("record name length {len} exceeds 1 KiB sanity cap")]
    NameTooLong { len: u32 },
    #[error("write payload size {size} exceeds 1 MiB sanity cap")]
    WriteTooLarge { size: u32 },
    #[error("unexpected EOF while reading {in_field}")]
    UnexpectedEof { in_field: &'static str },
}

impl ParseError {
    /// Whether this error is fatal (no resync possible). Streaming
    /// parsers abort on fatal errors; non-fatal errors trigger
    /// byte-by-byte resync to the next valid record magic.
    pub fn is_fatal(&self) -> bool {
        match self {
            ParseError::Io(_) => true,
            ParseError::BadHeaderMagic { .. }
            | ParseError::BadVersion { .. }
            | ParseError::NameTooLong { .. }
            | ParseError::WriteTooLarge { .. }
            | ParseError::UnexpectedEof { .. } => false,
        }
    }
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
    // Sliding 4-byte magic window. Read up to 4 bytes per attempt so
    // a BufReader buffer boundary cannot short-circuit EOF detection;
    // a real EOF returns 0 from the first read.
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
            if reader.read(&mut one).map_err(ParseError::Io)? == 0 {
                break;
            }
            window[3] = one[0];
            resyncs += 1;
            continue;
        }
        // Body-parse failures (NameTooLong, WriteTooLarge, EOF on a
        // file still being written) resync to the next valid magic
        // instead of aborting. `Io` errors still surface.
        have_window = false;
        match parse_one_record(&mut reader) {
            Ok(rec) => on_record(rec)?,
            Err(e) => {
                if e.is_fatal() {
                    return Err(e);
                }
                resyncs += 1;
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

/// Parse CLI args and run one of the query modes (`--addr`,
/// `--list`, `--ranked`, `--name`). All modes stream the trace.
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
#[path = "tests/rpcs3_attribute_tests.rs"]
mod tests;
