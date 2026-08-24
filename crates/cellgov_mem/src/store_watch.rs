//! Env-gated per-store watch on the commit-pipeline write path.
//!
//! Companion to the RPCS3-side `cellgov_store_watch.h` hook
//! (bridges/rpcs3-patch). When `CELLGOV_STORE_WATCH=<addr>:<len>` and
//! `CELLGOV_STORE_WATCH_PATH=<path>` are set, every guest write
//! landing in `[addr, addr+len)` is recorded as
//! `(step, pc, ea, width, value)` to the host file.
//!
//! Same binary format as the RPCS3 side so a single reader consumes
//! both logs.
//!
//! Firing from `GuestMemory::apply_commit`, the runtime batch-commits
//! at end-of-step granularity: `step` counts watch records (not PPU
//! instructions), `pc` is recorded as 0 (apply_commit is not PC-aware),
//! and `width` is the commit batch length in bytes. The `ea` field
//! still pinpoints the exact byte written.

#![allow(clippy::print_stderr)]

use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

struct WatchState {
    watch_addr: u64,
    watch_len: u64,
    writer: Mutex<BufWriter<File>>,
    step: Mutex<u64>,
}

static STATE: OnceLock<Option<WatchState>> = OnceLock::new();

/// Latch so a failing sink names itself once instead of per record.
static WRITE_FAILED: AtomicBool = AtomicBool::new(false);

fn init() -> Option<WatchState> {
    let spec = env::var("CELLGOV_STORE_WATCH").ok()?;
    let path = env::var("CELLGOV_STORE_WATCH_PATH").ok()?;
    if spec.is_empty() || path.is_empty() {
        return None;
    }
    let (addr_s, len_s) = spec.split_once(':')?;
    let addr_v = u64::from_str_radix(addr_s.trim_start_matches("0x"), 16).ok()?;
    let len_v = u64::from_str_radix(len_s.trim_start_matches("0x"), 16).ok()?;
    if len_v == 0 || len_v > 0x10000 {
        eprintln!("[cellgov] CELLGOV_STORE_WATCH len out of range (1..0x10000): 0x{len_v:x}");
        return None;
    }
    let file = File::create(&path).ok()?;
    let mut writer = BufWriter::new(file);
    // 16-byte header: 'CGSW' + version u32 = 1 + watch_addr u32 + watch_len u32
    writer.write_all(b"CGSW").ok()?;
    writer.write_all(&1u32.to_le_bytes()).ok()?;
    writer.write_all(&(addr_v as u32).to_le_bytes()).ok()?;
    writer.write_all(&(len_v as u32).to_le_bytes()).ok()?;
    writer.flush().ok()?;
    eprintln!("[cellgov] store-watch active: addr=0x{addr_v:x} len=0x{len_v:x} path={path}");
    Some(WatchState {
        watch_addr: addr_v,
        watch_len: len_v,
        writer: Mutex::new(writer),
        step: Mutex::new(0),
    })
}

fn state() -> Option<&'static WatchState> {
    STATE.get_or_init(init).as_ref()
}

/// One 28-byte record: `{ step u64, pc u32, ea u32, width u32, value u64 }`,
/// little-endian.
///
/// Packed whole so one `write_all` either lands the record or none of
/// it: the RPCS3-side reader walks fixed-size records from the header,
/// and a torn one shifts every later field.
fn pack_record(step: u64, pc: u32, ea: u64, width: u32, value: u64) -> [u8; 28] {
    let mut record = [0u8; 28];
    record[0..8].copy_from_slice(&step.to_le_bytes());
    record[8..12].copy_from_slice(&pc.to_le_bytes());
    record[12..16].copy_from_slice(&(ea as u32).to_le_bytes());
    record[16..20].copy_from_slice(&width.to_le_bytes());
    record[20..28].copy_from_slice(&value.to_le_bytes());
    record
}

/// Emit one watch record per byte-range that overlaps the configured
/// watch window. Records the FULL payload (not just the overlapping
/// subrange); readers intersect against the watch range themselves.
/// `value` carries the first 8 payload bytes (zero-padded for shorter
/// payloads, truncated for longer).
pub fn emit(pc: u32, ea: u64, bytes: &[u8]) {
    let Some(s) = state() else { return };
    if bytes.is_empty() {
        return;
    }
    let ea_end = ea.saturating_add(bytes.len() as u64);
    if ea_end <= s.watch_addr {
        return;
    }
    if ea >= s.watch_addr + s.watch_len {
        return;
    }
    let width = bytes.len() as u32;
    let mut value_buf = [0u8; 8];
    let take = bytes.len().min(8);
    value_buf[..take].copy_from_slice(&bytes[..take]);
    let value_u64 = u64::from_le_bytes(value_buf);

    // pc=0 from apply_commit: substitute the last PPU CIA executed
    // before yielding into the commit pipeline. Identifies the active
    // code region, not the exact storing instruction (stores batch
    // between flushes).
    let effective_pc = if pc == 0 {
        LAST_PPU_CIA.with(|c| c.get())
    } else {
        pc
    };

    let mut step_g = s.step.lock().expect("store-watch step mutex");
    let step = *step_g;
    *step_g = step.wrapping_add(1);
    drop(step_g);

    let record = pack_record(step, effective_pc, ea, width, value_u64);

    let mut writer = s.writer.lock().expect("store-watch writer mutex");
    // Per-record flush so a crash mid-run leaves the file readable up
    // to the last completed record.
    if let Err(e) = writer.write_all(&record).and_then(|()| writer.flush()) {
        // A dropped record makes every later one land at the wrong
        // offset for a reader counting from the header, so the operator
        // hears about it rather than reading a short capture as a
        // complete one.
        if !WRITE_FAILED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[cellgov] store-watch write failed at record {step}: {e};                  the capture is truncated from here"
            );
        }
    }
}

thread_local! {
    static LAST_PPU_CIA: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Stash the most recently executed PPU CIA. Called by the interpreter
/// dispatch loop on every instruction.
pub fn set_last_ppu_cia(pc: u32) {
    LAST_PPU_CIA.with(|c| c.set(pc));
}

#[cfg(test)]
#[path = "tests/store_watch_tests.rs"]
mod tests;
