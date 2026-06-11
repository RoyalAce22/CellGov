//! Env-gated per-call HLE return watch on guest PPU function entries.
//!
//! Configured by env var with NIDs the firmware exports. After PRX
//! load, [`register_nid_resolution`] resolves each NID to its OPD
//! entry PC. The PPU dispatch calls [`on_dispatch`] per instruction;
//! `state.pc` matches drive entry/exit records via a per-thread
//! in-flight stack of return PCs (snapshot of `lr` at entry).
//!
//! Caller contract: invoke [`is_active`] (or any state-touching API)
//! before the first dispatch so the [`OnceLock`] initializes and the
//! first watched instruction is not missed.
//!
//! [`totals`] surfaces `dropped_body_events` -- non-zero proves the
//! hook reached instruction dispatch even when no watched entry PC
//! was hit, distinguishing "hook never fired" from "hook fired but
//! never inside a watched scope."
//!
//! Env vars:
//!
//!   CELLGOV_HLE_RETURN_WATCH       Comma-separated hex NIDs.
//!   CELLGOV_HLE_RETURN_WATCH_PCS   Comma-separated `pc=name` for
//!                                  per-PRX entries whose NID is not
//!                                  unique across PRXes.
//!   CELLGOV_HLE_RETURN_WATCH_PATH  Output file path.
//!
//! File format (little-endian, no padding inside records):
//!
//!   [Header, 16 + 4*N bytes]
//!     magic     "CGHW"
//!     version   u32 = 1
//!     num_nids  u32 = N
//!     nid_i     u32, i in 0..N    (real NIDs followed by raw-PC
//!                                  synthetic IDs `pc | 0x80000000`)
//!
//!   [Resolution record], emitted once per watched ID at PRX load
//!   (raw-PC entries emitted at watch init):
//!     kind      u8 = 3
//!     nid       u32
//!     entry_pc  u32
//!     name_len  u8
//!     name      bytes\[name_len\]
//!
//!   [Entry record], emitted at each watched function entry:
//!     kind      u8 = 1
//!     record_no u64        (monotonic per file)
//!     nid       u32
//!     entry_pc  u32
//!     pc        u32        (same as entry_pc for sanity)
//!     lr        u32        (return PC after the call)
//!     r3..r10   u64 x 8
//!
//!   [Exit record], emitted when pc reaches a watched call's return PC:
//!     kind             u8 = 2
//!     record_no        u64
//!     nid              u32
//!     entry_record_no  u64        (paired Entry's record_no)
//!     pc               u32        (the return PC)
//!     r3               u64        (return value)

#![allow(clippy::print_stderr)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

const KIND_ENTRY: u8 = 1;
const KIND_EXIT: u8 = 2;
const KIND_RESOLUTION: u8 = 3;
/// Body-event `sc` inside a watched function (r11 + r3..r10).
const KIND_BODY_SYSCALL: u8 = 4;
/// Paired syscall-return point (r3 + entry record number).
const KIND_BODY_SYSCALL_RETURN: u8 = 5;
/// Body-event `bl` / `bctrl` / `blrl` inside a watched function.
const KIND_BODY_CALL: u8 = 6;

/// In-memory key for the resolved-entries map; Raw-PC and NID
/// keyspaces are disjoint.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum WatchKey {
    Nid(u32),
    RawPc(u32),
}

/// File-side state guarded by a single mutex so counter assignment
/// and the file append are atomic, and [`totals`] returns a
/// self-consistent snapshot.
struct WriterState {
    writer: BufWriter<File>,
    record_counter: u64,
    entry_total: u64,
    exit_total: u64,
    /// Body events executed outside any watched scope. Non-zero
    /// proves the hook reached dispatch even when `entry_total == 0`.
    dropped_body_events: u64,
}

struct WatchState {
    watched_nids: Vec<u32>,
    resolved: Mutex<BTreeMap<WatchKey, ResolvedEntry>>,
    writer: Mutex<WriterState>,
}

struct ResolvedEntry {
    on_wire_nid: u32,
    entry_pc: u32,
    #[allow(dead_code)]
    name: String,
}

#[derive(Clone, Copy)]
struct InFlightCall {
    on_wire_nid: u32,
    return_pc: u32,
    entry_record_no: u64,
}

#[derive(Clone, Copy)]
struct PendingSyscallReturn {
    return_pc: u32,
    syscall_num: u32,
    in_flight_on_wire_nid: u32,
    entry_record_no: u64,
}

static STATE: OnceLock<Option<WatchState>> = OnceLock::new();

thread_local! {
    static IN_FLIGHT: RefCell<Vec<InFlightCall>> = const { RefCell::new(Vec::new()) };
    static PENDING_SYSCALL_RETURNS: RefCell<Vec<PendingSyscallReturn>> = const { RefCell::new(Vec::new()) };
}

fn init() -> Option<WatchState> {
    let path = env::var("CELLGOV_HLE_RETURN_WATCH_PATH").ok()?;
    if path.is_empty() {
        return None;
    }
    let spec = env::var("CELLGOV_HLE_RETURN_WATCH")
        .ok()
        .unwrap_or_default();
    let pcs_spec = env::var("CELLGOV_HLE_RETURN_WATCH_PCS")
        .ok()
        .unwrap_or_default();
    let mut watched_nids: Vec<u32> = Vec::new();
    for tok in spec.split(',') {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            continue;
        }
        let body = trimmed.trim_start_matches("0x").trim_start_matches("0X");
        match u32::from_str_radix(body, 16) {
            Ok(n) => watched_nids.push(n),
            Err(e) => {
                eprintln!("[cellgov] CELLGOV_HLE_RETURN_WATCH: cannot parse {trimmed:?}: {e}");
                return None;
            }
        }
    }
    let mut raw_pc_watches: Vec<(u32, String)> = Vec::new();
    for tok in pcs_spec.split(',') {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((pc_s, name_s)) = trimmed.split_once('=') else {
            eprintln!(
                "[cellgov] CELLGOV_HLE_RETURN_WATCH_PCS: expected <pc>=<name>, got {trimmed:?}"
            );
            return None;
        };
        let pc_body = pc_s.trim_start_matches("0x").trim_start_matches("0X");
        let pc = match u32::from_str_radix(pc_body, 16) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[cellgov] CELLGOV_HLE_RETURN_WATCH_PCS: cannot parse PC {pc_s:?}: {e}");
                return None;
            }
        };
        let synthetic_on_wire = pc | 0x8000_0000;
        debug_assert!(
            !watched_nids.contains(&synthetic_on_wire),
            "raw-PC synthetic on-wire ID 0x{synthetic_on_wire:08x} collides with a watched real NID; very rare but real on-wire ambiguity -- pick a different PC or drop the colliding NID"
        );
        raw_pc_watches.push((pc, name_s.to_string()));
    }
    if watched_nids.is_empty() && raw_pc_watches.is_empty() {
        eprintln!("[cellgov] hle-return-watch: no NIDs or raw PCs configured");
        return None;
    }

    // On-wire ID list: real NIDs first, then raw-PC synthetic IDs.
    let mut on_wire_ids: Vec<u32> = watched_nids.clone();
    for (pc, _) in &raw_pc_watches {
        on_wire_ids.push(*pc | 0x8000_0000);
    }

    // Build header in one buffer so `write_all` is the atomic
    // boundary against a torn header.
    let mut header: Vec<u8> = Vec::with_capacity(16 + 4 * on_wire_ids.len());
    header.extend_from_slice(b"CGHW");
    header.extend_from_slice(&1u32.to_le_bytes());
    header.extend_from_slice(&(on_wire_ids.len() as u32).to_le_bytes());
    for nid in &on_wire_ids {
        header.extend_from_slice(&nid.to_le_bytes());
    }

    let file = match File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[cellgov] hle-return-watch: cannot create {path}: {e}");
            return None;
        }
    };
    let mut writer = BufWriter::new(file);
    if let Err(e) = writer.write_all(&header) {
        eprintln!("[cellgov] hle-return-watch: header write to {path} failed: {e}");
        return None;
    }

    eprintln!(
        "[cellgov] hle-return-watch active: {} entries path={}",
        on_wire_ids.len(),
        path,
    );
    for nid in &on_wire_ids {
        eprintln!("[cellgov] hle-return-watch watching NID 0x{nid:08x}");
    }
    let mut resolved: BTreeMap<WatchKey, ResolvedEntry> = BTreeMap::new();
    for (pc, name) in &raw_pc_watches {
        let on_wire = *pc | 0x8000_0000;
        resolved.insert(
            WatchKey::RawPc(*pc),
            ResolvedEntry {
                on_wire_nid: on_wire,
                entry_pc: *pc,
                name: name.clone(),
            },
        );
        eprintln!(
            "[cellgov] hle-return-watch raw-PC entry registered: 0x{pc:08x} ({name}) on_wire=0x{on_wire:08x}"
        );
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(255) as u8;
        let mut rec: Vec<u8> = Vec::with_capacity(1 + 4 + 4 + 1 + usize::from(name_len));
        rec.push(KIND_RESOLUTION);
        rec.extend_from_slice(&on_wire.to_le_bytes());
        rec.extend_from_slice(&pc.to_le_bytes());
        rec.push(name_len);
        rec.extend_from_slice(&name_bytes[..usize::from(name_len)]);
        if let Err(e) = writer.write_all(&rec) {
            eprintln!("[cellgov] hle-return-watch: raw-PC resolution write to {path} failed: {e}");
            return None;
        }
    }
    if let Err(e) = writer.flush() {
        eprintln!("[cellgov] hle-return-watch: init flush to {path} failed: {e}");
        return None;
    }

    Some(WatchState {
        watched_nids,
        resolved: Mutex::new(resolved),
        writer: Mutex::new(WriterState {
            writer,
            record_counter: 0,
            entry_total: 0,
            exit_total: 0,
            dropped_body_events: 0,
        }),
    })
}

fn state() -> Option<&'static WatchState> {
    STATE.get_or_init(init).as_ref()
}

/// True when the env-gated instrument is active.
pub fn is_active() -> bool {
    state().is_some()
}

/// Real NIDs the instrument is watching; empty when inactive.
/// Raw-PC watches are pre-resolved at init and are NOT included
/// here -- the caller (firmware PRX-load resolution) should not
/// look them up in the export table.
pub fn watched_nids() -> Vec<u32> {
    state().map(|s| s.watched_nids.clone()).unwrap_or_default()
}

/// Register an entry-PC resolution for a watched NID; writes one
/// resolution record on the first call for a given NID.
pub fn register_nid_resolution(nid: u32, name: &str, entry_pc: u32) {
    let Some(s) = state() else { return };
    if !s.watched_nids.contains(&nid) {
        return;
    }
    let key = WatchKey::Nid(nid);
    let mut map = s.resolved.lock().expect("hle_watch resolved mutex");
    if map.contains_key(&key) {
        return;
    }
    map.insert(
        key,
        ResolvedEntry {
            on_wire_nid: nid,
            entry_pc,
            name: name.to_string(),
        },
    );
    drop(map);
    eprintln!(
        "[cellgov] hle-return-watch resolved NID 0x{nid:08x} ({name}) entry_pc=0x{entry_pc:08x}"
    );
    let mut w = s.writer.lock().expect("hle_watch writer");
    let _ = w.writer.write_all(&[KIND_RESOLUTION]);
    let _ = w.writer.write_all(&nid.to_le_bytes());
    let _ = w.writer.write_all(&entry_pc.to_le_bytes());
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len().min(255) as u8;
    let _ = w.writer.write_all(&[name_len]);
    let _ = w.writer.write_all(&name_bytes[..usize::from(name_len)]);
    let _ = w.writer.flush();
}

/// Per-instruction hook; fast-paths to a no-op when inactive.
#[inline]
pub fn on_dispatch(pc: u32, gpr: &[u64; 32], lr: u64) {
    if STATE.get().is_none_or(Option::is_none) {
        return;
    }
    on_dispatch_slow(pc, gpr, lr);
}

#[inline(never)]
fn on_dispatch_slow(pc: u32, gpr: &[u64; 32], lr: u64) {
    let Some(s) = state() else { return };

    // Process exit before entry so a degenerate PC where outer
    // return_pc == new entry pops the outer frame first.
    //
    // PC-equality limitation: when `lr == entry_pc`, the return
    // visit pops then matches entry at the same PC, emitting a
    // phantom entry. Analyzers should treat back-to-back entries
    // with no body events as suspect on PCs that host this pattern.
    let exit_match: Option<InFlightCall> = IN_FLIGHT.with(|stack| {
        let st = stack.borrow();
        st.last().filter(|c| c.return_pc == pc).copied()
    });
    let entry_match: Option<(u32, u32)> = {
        let map = s.resolved.lock().expect("hle_watch resolved mutex");
        if map.is_empty() {
            None
        } else {
            map.values()
                .find(|r| r.entry_pc == pc)
                .map(|r| (r.on_wire_nid, r.entry_pc))
        }
    };

    if let Some(call) = exit_match {
        let mut w = s.writer.lock().expect("hle_watch writer");
        let record_no = w.record_counter;
        w.record_counter = w.record_counter.wrapping_add(1);
        w.exit_total = w.exit_total.wrapping_add(1);
        let _ = w.writer.write_all(&[KIND_EXIT]);
        let _ = w.writer.write_all(&record_no.to_le_bytes());
        let _ = w.writer.write_all(&call.on_wire_nid.to_le_bytes());
        let _ = w.writer.write_all(&call.entry_record_no.to_le_bytes());
        let _ = w.writer.write_all(&pc.to_le_bytes());
        let _ = w.writer.write_all(&gpr[3].to_le_bytes());
        let _ = w.writer.flush();
        drop(w);
        IN_FLIGHT.with(|stack| {
            stack.borrow_mut().pop();
        });
    }

    if let Some((on_wire_nid, entry_pc)) = entry_match {
        // PS3 guest addresses fit in u32; lr is 64-bit. The on-wire
        // format narrows to u32, so guard the cast here.
        debug_assert!(
            lr >> 32 == 0,
            "PPU lr 0x{lr:016x} exceeds 32-bit; instrument's on-wire format is u32"
        );
        let lr32 = lr as u32;
        let mut w = s.writer.lock().expect("hle_watch writer");
        let record_no = w.record_counter;
        w.record_counter = w.record_counter.wrapping_add(1);
        w.entry_total = w.entry_total.wrapping_add(1);
        let _ = w.writer.write_all(&[KIND_ENTRY]);
        let _ = w.writer.write_all(&record_no.to_le_bytes());
        let _ = w.writer.write_all(&on_wire_nid.to_le_bytes());
        let _ = w.writer.write_all(&entry_pc.to_le_bytes());
        let _ = w.writer.write_all(&pc.to_le_bytes());
        let _ = w.writer.write_all(&lr32.to_le_bytes());
        for r in &gpr[3..=10] {
            let _ = w.writer.write_all(&r.to_le_bytes());
        }
        let _ = w.writer.flush();
        drop(w);
        IN_FLIGHT.with(|stack| {
            stack.borrow_mut().push(InFlightCall {
                on_wire_nid,
                return_pc: lr32,
                entry_record_no: record_no,
            });
        });
    }

    let pending_return = PENDING_SYSCALL_RETURNS.with(|stack| {
        let st = stack.borrow();
        st.last().filter(|p| p.return_pc == pc).copied()
    });
    if let Some(pending) = pending_return {
        let mut w = s.writer.lock().expect("hle_watch writer");
        let record_no = w.record_counter;
        w.record_counter = w.record_counter.wrapping_add(1);
        let _ = w.writer.write_all(&[KIND_BODY_SYSCALL_RETURN]);
        let _ = w.writer.write_all(&record_no.to_le_bytes());
        let _ = w
            .writer
            .write_all(&pending.in_flight_on_wire_nid.to_le_bytes());
        let _ = w.writer.write_all(&pending.entry_record_no.to_le_bytes());
        let _ = w.writer.write_all(&pending.syscall_num.to_le_bytes());
        let _ = w.writer.write_all(&pc.to_le_bytes());
        let _ = w.writer.write_all(&gpr[3].to_le_bytes());
        let _ = w.writer.flush();
        drop(w);
        PENDING_SYSCALL_RETURNS.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Body event hook for `sc`. Records a body-syscall entry and queues
/// a return for `pc + 4` when a watched function is in flight; else
/// increments `dropped_body_events`.
#[inline]
pub fn on_syscall(pc: u32, gpr: &[u64; 32]) {
    if STATE.get().is_none_or(Option::is_none) {
        return;
    }
    on_syscall_slow(pc, gpr);
}

#[inline(never)]
fn on_syscall_slow(pc: u32, gpr: &[u64; 32]) {
    let Some(s) = state() else { return };
    let innermost = IN_FLIGHT.with(|stack| stack.borrow().last().copied());
    let Some(call) = innermost else {
        let mut w = s.writer.lock().expect("hle_watch writer");
        w.dropped_body_events = w.dropped_body_events.wrapping_add(1);
        return;
    };
    let syscall_num = gpr[11] as u32;
    let mut w = s.writer.lock().expect("hle_watch writer");
    let record_no = w.record_counter;
    w.record_counter = w.record_counter.wrapping_add(1);
    let _ = w.writer.write_all(&[KIND_BODY_SYSCALL]);
    let _ = w.writer.write_all(&record_no.to_le_bytes());
    let _ = w.writer.write_all(&call.on_wire_nid.to_le_bytes());
    let _ = w.writer.write_all(&call.entry_record_no.to_le_bytes());
    let _ = w.writer.write_all(&syscall_num.to_le_bytes());
    let _ = w.writer.write_all(&pc.to_le_bytes());
    for r in &gpr[3..=10] {
        let _ = w.writer.write_all(&r.to_le_bytes());
    }
    let _ = w.writer.flush();
    drop(w);
    PENDING_SYSCALL_RETURNS.with(|stack| {
        stack.borrow_mut().push(PendingSyscallReturn {
            return_pc: pc.wrapping_add(4),
            syscall_num,
            in_flight_on_wire_nid: call.on_wire_nid,
            entry_record_no: call.entry_record_no,
        });
    });
}

/// Body event hook for `bl` / `bctrl` / `blrl`. Caller passes the
/// resolved target. Same drop accounting as [`on_syscall`].
#[inline]
pub fn on_branch_link(pc: u32, gpr: &[u64; 32], target: u32) {
    if STATE.get().is_none_or(Option::is_none) {
        return;
    }
    on_branch_link_slow(pc, gpr, target);
}

#[inline(never)]
fn on_branch_link_slow(pc: u32, gpr: &[u64; 32], target: u32) {
    let Some(s) = state() else { return };
    let innermost = IN_FLIGHT.with(|stack| stack.borrow().last().copied());
    let Some(call) = innermost else {
        let mut w = s.writer.lock().expect("hle_watch writer");
        w.dropped_body_events = w.dropped_body_events.wrapping_add(1);
        return;
    };
    let mut w = s.writer.lock().expect("hle_watch writer");
    let record_no = w.record_counter;
    w.record_counter = w.record_counter.wrapping_add(1);
    let _ = w.writer.write_all(&[KIND_BODY_CALL]);
    let _ = w.writer.write_all(&record_no.to_le_bytes());
    let _ = w.writer.write_all(&call.on_wire_nid.to_le_bytes());
    let _ = w.writer.write_all(&call.entry_record_no.to_le_bytes());
    let _ = w.writer.write_all(&pc.to_le_bytes());
    let _ = w.writer.write_all(&target.to_le_bytes());
    for r in &gpr[3..=10] {
        let _ = w.writer.write_all(&r.to_le_bytes());
    }
    let _ = w.writer.flush();
}

/// End-of-run `(entry_total, exit_total, dropped_body_events)`; `None`
/// when inactive.
pub fn totals() -> Option<(u64, u64, u64)> {
    let s = state()?;
    let w = s.writer.lock().expect("hle_watch writer");
    Some((w.entry_total, w.exit_total, w.dropped_body_events))
}
