//! `dev rpcs3-attribute` subcommand: read an RPCS3 HLE trace produced by
//! the patched build (`bridges/rpcs3-patch/0002-cellgov-hle-trace.patch`)
//! and answer "which HLE call wrote this guest address?"
//!
//! `cellgov_ppu::differential::rpcs3_hle_trace` decodes the trace; this
//! command renders what it finds.
//!
//! # Examples
//!
//! ```text
//! cellgov dev rpcs3-attribute --trace title.htrc --addr 0x101e3cb8
//! ```

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use cellgov_ppu::differential::rpcs3_hle_trace::{
    HleCallRecord, HleTraceEvent, HleTraceReader, HleWriteTally,
};

use super::exit::CommandError;
use super::parse::Rpcs3AttributeArgs;

/// Streams an RPCS3 trace through the selected query mode.
///
/// # Errors
///
/// Returns an error in these cases:
///
/// - The trace path does not name a file.
/// - The trace cannot be parsed.
pub fn run(args: &Rpcs3AttributeArgs) -> Result<(), CommandError> {
    let path: &Path = &args.trace;
    let trace_path = path.display();
    if !path.is_file() {
        return Err(CommandError::failed(format!(
            "trace file not found: {trace_path} (did the patched runner produce one?)"
        )));
    }

    let filters = Filters {
        list: args.list,
        ranked: args.ranked,
        addr: args.addr.map(|addr| (addr, args.len.unwrap_or(1))),
        name: args.name.as_deref(),
    };
    let scan = scan_trace(path, &filters)?;
    if scan.resyncs > 0 {
        eprintln!(
            "rpcs3-attribute: skipped {} byte(s) of corrupted/partial trace data while resyncing",
            scan.resyncs,
        );
    }

    eprintln!(
        "rpcs3-attribute: streamed {} record(s) from {trace_path}",
        scan.total
    );

    if filters.ranked {
        println!("{:>8}  {:>8}  name", "writes", "calls");
        for row in scan.tally.ranked() {
            println!("{:>8}  {:>8}  {}", row.writes, row.calls, row.name);
        }
    }

    if let Some((addr, len)) = filters.addr {
        let end = addr.saturating_add(len);
        print_hits(
            &scan.hits,
            &format!(
                "no records wrote to [0x{addr:016x}, 0x{end:016x}). Address may be untouched, or the watch list the trace was captured under did not cover it."
            ),
            &format!(
                "{} record(s) wrote into [0x{addr:016x}, 0x{end:016x}) in chronological order:",
                scan.hits.len(),
            ),
        );
    }

    if let Some(needle) = filters.name {
        print_hits(
            &scan.name_hits,
            &format!("no records matched name substring {needle:?}"),
            &format!(
                "{} record(s) matched name substring {needle:?} in chronological order:",
                scan.name_hits.len(),
            ),
        );
    }
    Ok(())
}

/// The query modes the scan serves.
struct Filters<'a> {
    list: bool,
    ranked: bool,
    /// Start and length of the queried address range.
    addr: Option<(u64, u64)>,
    name: Option<&'a str>,
}

/// What one pass over the trace collected.
struct Scan {
    total: usize,
    hits: Vec<HleCallRecord>,
    name_hits: Vec<HleCallRecord>,
    tally: HleWriteTally,
    /// Bytes skipped plus records dropped while resyncing.
    resyncs: usize,
}

/// Stream the trace at `path` once, printing each record under `--list`
/// and collecting what the other modes report.
fn scan_trace(path: &Path, filters: &Filters<'_>) -> Result<Scan, CommandError> {
    let mut scan = Scan {
        total: 0,
        hits: Vec::new(),
        name_hits: Vec::new(),
        tally: HleWriteTally::default(),
        resyncs: 0,
    };
    let parse_failed = |error| CommandError::failed(format!("failed to parse trace: {error}"));
    let file = File::open(path).map_err(|error| {
        parse_failed(cellgov_ppu::differential::rpcs3_hle_trace::HleTraceError::Io(error))
    })?;
    let reader =
        HleTraceReader::new(BufReader::with_capacity(1 << 20, file)).map_err(parse_failed)?;
    for event in reader {
        let rec = match event.map_err(parse_failed)? {
            HleTraceEvent::Record(rec) => rec,
            HleTraceEvent::SkippedBytes(n) => {
                scan.resyncs += n;
                continue;
            }
            HleTraceEvent::DroppedRecord(_) => {
                scan.resyncs += 1;
                continue;
            }
        };
        scan.total += 1;
        if scan.total.is_multiple_of(1_000_000) {
            eprintln!("rpcs3-attribute: streamed {} records...", scan.total);
        }
        if filters.list {
            print_record(&rec, "");
        }
        if filters.ranked {
            scan.tally.add(&rec);
        }
        if filters
            .addr
            .is_some_and(|(addr, len)| rec.writes_into(addr, len))
        {
            scan.hits.push(rec.clone());
        }
        if filters.name.is_some_and(|needle| rec.name.contains(needle)) {
            scan.name_hits.push(rec);
        }
    }
    Ok(scan)
}

/// Print `empty` when no record matched, else `header` and each record
/// indented.
fn print_hits(hits: &[HleCallRecord], empty: &str, header: &str) {
    if hits.is_empty() {
        println!("{empty}");
        return;
    }
    println!("{header}");
    for rec in hits {
        print_record(rec, "  ");
    }
}

fn print_record(rec: &HleCallRecord, indent: &str) {
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
