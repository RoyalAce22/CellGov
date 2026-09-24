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

    let want_list = args.list;
    let want_ranked = args.ranked;
    let name_filter = args.name.clone();
    let addr_filter: Option<(u64, u64)> = args.addr.map(|addr| (addr, args.len.unwrap_or(1)));

    let mut total_records = 0usize;
    let mut hits: Vec<HleCallRecord> = Vec::new();
    let mut name_hits: Vec<HleCallRecord> = Vec::new();
    let mut tally = HleWriteTally::default();
    let mut resyncs = 0usize;

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
                resyncs += n;
                continue;
            }
            HleTraceEvent::DroppedRecord(_) => {
                resyncs += 1;
                continue;
            }
        };
        total_records += 1;
        if total_records.is_multiple_of(1_000_000) {
            eprintln!("rpcs3-attribute: streamed {total_records} records...");
        }
        if want_list {
            print_record(&rec, "");
        }
        if want_ranked {
            tally.add(&rec);
        }
        if let Some((addr, len)) = addr_filter {
            if rec.writes_into(addr, len) {
                hits.push(rec.clone());
            }
        }
        if let Some(needle) = name_filter.as_deref() {
            if rec.name.contains(needle) {
                name_hits.push(rec);
            }
        }
    }
    if resyncs > 0 {
        eprintln!(
            "rpcs3-attribute: skipped {resyncs} byte(s) of corrupted/partial trace data while resyncing",
        );
    }

    eprintln!("rpcs3-attribute: streamed {total_records} record(s) from {trace_path}",);

    if want_ranked {
        println!("{:>8}  {:>8}  name", "writes", "calls");
        for row in tally.ranked() {
            println!("{:>8}  {:>8}  {}", row.writes, row.calls, row.name);
        }
    }

    if let Some((addr, len)) = addr_filter {
        if hits.is_empty() {
            println!(
                "no records wrote to [0x{addr:016x}, 0x{:016x}). Address may be untouched, or the watch list the trace was captured under did not cover it.",
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
    Ok(())
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
