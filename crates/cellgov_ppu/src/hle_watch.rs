//! Env-gated per-call HLE return watch on guest PPU function entries.
//!
//! Configured by env var with a set of NIDs the firmware exports.
//! After PRX load, [`register_nid_resolution`] resolves each watched
//! NID to its OPD entry PC and records the resolution in the output
//! file. The PPU dispatch then calls [`on_dispatch`] per instruction;
//! when `state.pc` matches a watched entry PC, an "entry" record is
//! written carrying `r3..r10` and the link register at entry. A
//! per-thread in-flight stack tracks each call's return PC (snapshot
//! of `lr` at entry); when a later instruction's `state.pc` matches
//! the top of stack, an "exit" record is written carrying `r3` (the
//! return value).
//!
//! Coverage and liveness:
//!
//! - Hook fires at the **firmware-PRX function body's first
//!   instruction**, reached after the title's import stub
//!   trampolines through the OPD. Misses any path that does not go
//!   through that entry PC (e.g. an inlined PRX implementation,
//!   which does not exist in the shipped firmware PRX layout but is
//!   listed here for completeness).
//! - "No entry record for NID X" means the hook did not see PC reach
//!   the resolved entry PC. It does **not** mean the title did not
//!   call NID X. A reader analyzing the output must demonstrate the
//!   hook fired at least once on a known-live NID before reading any
//!   "zero records" row as signal -- the silent-failure trap is
//!   identical to the patch-0003 store_watch caveat.
//! - "NID X did not resolve" (no resolution record in the output
//!   file) means the export table did not contain NID X. Possible
//!   causes: NID is not exported by any loaded firmware PRX, NID hex
//!   was mistyped on the env var, or the dependency graph did not
//!   load the exporting PRX. Distinct from "resolved but not seen."
//!
//! Env vars:
//!
//!   CELLGOV_HLE_RETURN_WATCH       Comma-separated hex NIDs
//!                                  (e.g. `0x32267a31,0x15bae46b`).
//!   CELLGOV_HLE_RETURN_WATCH_PCS   Comma-separated raw entry PCs
//!                                  with names, in `pc=name`
//!                                  form (e.g.
//!                                  `0x10432bc8=libsysmodule_module_start`).
//!                                  For per-PRX entry points
//!                                  (`module_start` / `module_stop`)
//!                                  whose NID is reused across PRXes
//!                                  and so does not key uniquely in
//!                                  the merged export table. PCs are
//!                                  registered immediately at watch
//!                                  init; PRX-load resolution does
//!                                  not touch them.
//!   CELLGOV_HLE_RETURN_WATCH_PATH  Output file path.
//!
//! File format (little-endian, no padding inside records):
//!
//!   [Header, 16 + 4*N bytes]
//!     magic     "CGHW"
//!     version   u32 = 1
//!     num_nids  u32 = N
//!     nid_i     u32, i in 0..N
//!
//!   [Resolution record], emitted once per watched NID at PRX load:
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
/// Body event: syscall (`sc`) executed while inside a watched
/// function. Carries syscall number (r11) and arg regs (r3..r10).
const KIND_BODY_SYSCALL: u8 = 4;
/// Body event: syscall return point reached. Carries the syscall
/// return value (r3) and the paired syscall-entry record number.
const KIND_BODY_SYSCALL_RETURN: u8 = 5;
/// Body event: branch-with-link (`bl` / `bctrl` / `blrl`) executed
/// while inside a watched function. Carries the target PC (if
/// statically known from the encoding) and arg regs (r3..r10).
const KIND_BODY_CALL: u8 = 6;

struct WatchState {
    watched: Vec<u32>,
    resolved: Mutex<BTreeMap<u32, ResolvedEntry>>,
    writer: Mutex<BufWriter<File>>,
    record_counter: Mutex<u64>,
    entry_total: Mutex<u64>,
    exit_total: Mutex<u64>,
}

struct ResolvedEntry {
    entry_pc: u32,
    #[allow(dead_code)]
    name: String,
}

#[derive(Clone, Copy)]
struct InFlightCall {
    nid: u32,
    return_pc: u32,
    entry_record_no: u64,
}

#[derive(Clone, Copy)]
struct PendingSyscallReturn {
    return_pc: u32,
    syscall_num: u32,
    in_flight_nid: u32,
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
    let mut watched: Vec<u32> = Vec::new();
    for tok in spec.split(',') {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            continue;
        }
        let body = trimmed.trim_start_matches("0x").trim_start_matches("0X");
        match u32::from_str_radix(body, 16) {
            Ok(n) => watched.push(n),
            Err(e) => {
                eprintln!("[cellgov] CELLGOV_HLE_RETURN_WATCH: cannot parse {trimmed:?}: {e}");
                return None;
            }
        }
    }
    // Raw-PC entries become resolutions keyed by synthetic NID `pc | 0x80000000`.
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
        raw_pc_watches.push((pc, name_s.to_string()));
        let synthetic_nid = pc | 0x8000_0000;
        if !watched.contains(&synthetic_nid) {
            watched.push(synthetic_nid);
        }
    }
    if watched.is_empty() {
        eprintln!("[cellgov] hle-return-watch: no NIDs or raw PCs configured");
        return None;
    }
    let file = File::create(&path).ok()?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"CGHW").ok()?;
    writer.write_all(&1u32.to_le_bytes()).ok()?;
    writer
        .write_all(&(watched.len() as u32).to_le_bytes())
        .ok()?;
    for nid in &watched {
        writer.write_all(&nid.to_le_bytes()).ok()?;
    }
    writer.flush().ok()?;
    eprintln!(
        "[cellgov] hle-return-watch active: {} entries path={}",
        watched.len(),
        path,
    );
    for nid in &watched {
        eprintln!("[cellgov] hle-return-watch watching NID 0x{nid:08x}");
    }
    let mut resolved: BTreeMap<u32, ResolvedEntry> = BTreeMap::new();
    for (pc, name) in &raw_pc_watches {
        let synthetic_nid = *pc | 0x8000_0000;
        resolved.insert(
            synthetic_nid,
            ResolvedEntry {
                entry_pc: *pc,
                name: name.clone(),
            },
        );
        eprintln!(
            "[cellgov] hle-return-watch raw-PC entry registered: 0x{pc:08x} ({name}) synthetic_nid=0x{synthetic_nid:08x}"
        );
        let _ = writer.write_all(&[KIND_RESOLUTION]);
        let _ = writer.write_all(&synthetic_nid.to_le_bytes());
        let _ = writer.write_all(&pc.to_le_bytes());
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(255) as u8;
        let _ = writer.write_all(&[name_len]);
        let _ = writer.write_all(&name_bytes[..usize::from(name_len)]);
    }
    if !raw_pc_watches.is_empty() {
        let _ = writer.flush();
    }
    Some(WatchState {
        watched,
        resolved: Mutex::new(resolved),
        writer: Mutex::new(writer),
        record_counter: Mutex::new(0),
        entry_total: Mutex::new(0),
        exit_total: Mutex::new(0),
    })
}

fn state() -> Option<&'static WatchState> {
    STATE.get_or_init(init).as_ref()
}

/// True when the env-gated instrument is active.
pub fn is_active() -> bool {
    state().is_some()
}

/// NID list the instrument is watching; empty when inactive.
pub fn watched_nids() -> Vec<u32> {
    state().map(|s| s.watched.clone()).unwrap_or_default()
}

/// Register an entry-PC resolution for a watched NID; writes one
/// resolution record on the first call for a given NID.
pub fn register_nid_resolution(nid: u32, name: &str, entry_pc: u32) {
    let Some(s) = state() else { return };
    if !s.watched.contains(&nid) {
        return;
    }
    let mut map = s.resolved.lock().expect("hle_watch resolved mutex");
    if map.contains_key(&nid) {
        return;
    }
    map.insert(
        nid,
        ResolvedEntry {
            entry_pc,
            name: name.to_string(),
        },
    );
    drop(map);
    eprintln!(
        "[cellgov] hle-return-watch resolved NID 0x{nid:08x} ({name}) entry_pc=0x{entry_pc:08x}"
    );
    let mut writer = s.writer.lock().expect("hle_watch writer mutex");
    let _ = writer.write_all(&[KIND_RESOLUTION]);
    let _ = writer.write_all(&nid.to_le_bytes());
    let _ = writer.write_all(&entry_pc.to_le_bytes());
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len().min(255) as u8;
    let _ = writer.write_all(&[name_len]);
    let _ = writer.write_all(&name_bytes[..usize::from(name_len)]);
    let _ = writer.flush();
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
    let entry_match: Option<(u32, u32)> = {
        let map = s.resolved.lock().expect("hle_watch resolved mutex");
        if map.is_empty() {
            None
        } else {
            map.iter()
                .find(|(_, r)| r.entry_pc == pc)
                .map(|(nid, r)| (*nid, r.entry_pc))
        }
    };
    let exit_match: Option<InFlightCall> = IN_FLIGHT.with(|stack| {
        let st = stack.borrow();
        st.last().filter(|c| c.return_pc == pc).copied()
    });
    if let Some((nid, entry_pc)) = entry_match {
        let lr32 = lr as u32;
        let mut counter = s.record_counter.lock().expect("hle_watch counter");
        let record_no = *counter;
        *counter = counter.wrapping_add(1);
        drop(counter);
        let mut writer = s.writer.lock().expect("hle_watch writer mutex");
        let _ = writer.write_all(&[KIND_ENTRY]);
        let _ = writer.write_all(&record_no.to_le_bytes());
        let _ = writer.write_all(&nid.to_le_bytes());
        let _ = writer.write_all(&entry_pc.to_le_bytes());
        let _ = writer.write_all(&pc.to_le_bytes());
        let _ = writer.write_all(&lr32.to_le_bytes());
        for r in &gpr[3..=10] {
            let _ = writer.write_all(&r.to_le_bytes());
        }
        let _ = writer.flush();
        drop(writer);
        let mut tot = s.entry_total.lock().expect("hle_watch entry_total");
        *tot = tot.wrapping_add(1);
        drop(tot);
        IN_FLIGHT.with(|stack| {
            stack.borrow_mut().push(InFlightCall {
                nid,
                return_pc: lr32,
                entry_record_no: record_no,
            });
        });
    }
    if let Some(call) = exit_match {
        let mut counter = s.record_counter.lock().expect("hle_watch counter");
        let record_no = *counter;
        *counter = counter.wrapping_add(1);
        drop(counter);
        let mut writer = s.writer.lock().expect("hle_watch writer mutex");
        let _ = writer.write_all(&[KIND_EXIT]);
        let _ = writer.write_all(&record_no.to_le_bytes());
        let _ = writer.write_all(&call.nid.to_le_bytes());
        let _ = writer.write_all(&call.entry_record_no.to_le_bytes());
        let _ = writer.write_all(&pc.to_le_bytes());
        let _ = writer.write_all(&gpr[3].to_le_bytes());
        let _ = writer.flush();
        drop(writer);
        let mut tot = s.exit_total.lock().expect("hle_watch exit_total");
        *tot = tot.wrapping_add(1);
        drop(tot);
        IN_FLIGHT.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
    let pending_return = PENDING_SYSCALL_RETURNS.with(|stack| {
        let st = stack.borrow();
        st.last().filter(|p| p.return_pc == pc).copied()
    });
    if let Some(pending) = pending_return {
        let mut counter = s.record_counter.lock().expect("hle_watch counter");
        let record_no = *counter;
        *counter = counter.wrapping_add(1);
        drop(counter);
        let mut writer = s.writer.lock().expect("hle_watch writer mutex");
        let _ = writer.write_all(&[KIND_BODY_SYSCALL_RETURN]);
        let _ = writer.write_all(&record_no.to_le_bytes());
        let _ = writer.write_all(&pending.in_flight_nid.to_le_bytes());
        let _ = writer.write_all(&pending.entry_record_no.to_le_bytes());
        let _ = writer.write_all(&pending.syscall_num.to_le_bytes());
        let _ = writer.write_all(&pc.to_le_bytes());
        let _ = writer.write_all(&gpr[3].to_le_bytes());
        let _ = writer.flush();
        drop(writer);
        PENDING_SYSCALL_RETURNS.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Body event hook for `sc`; records a body-syscall entry when a
/// watched function is in flight and queues a return record for
/// `pc + 4`.
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
    let Some(call) = innermost else { return };
    let syscall_num = gpr[11] as u32;
    let mut counter = s.record_counter.lock().expect("hle_watch counter");
    let record_no = *counter;
    *counter = counter.wrapping_add(1);
    drop(counter);
    let mut writer = s.writer.lock().expect("hle_watch writer mutex");
    let _ = writer.write_all(&[KIND_BODY_SYSCALL]);
    let _ = writer.write_all(&record_no.to_le_bytes());
    let _ = writer.write_all(&call.nid.to_le_bytes());
    let _ = writer.write_all(&call.entry_record_no.to_le_bytes());
    let _ = writer.write_all(&syscall_num.to_le_bytes());
    let _ = writer.write_all(&pc.to_le_bytes());
    for r in &gpr[3..=10] {
        let _ = writer.write_all(&r.to_le_bytes());
    }
    let _ = writer.flush();
    drop(writer);
    PENDING_SYSCALL_RETURNS.with(|stack| {
        stack.borrow_mut().push(PendingSyscallReturn {
            return_pc: pc.wrapping_add(4),
            syscall_num,
            in_flight_nid: call.nid,
            entry_record_no: call.entry_record_no,
        });
    });
}

/// Body event hook for `bl` / `bctrl` / `blrl`; caller passes the
/// resolved target (CTR for `bctrl`, LR for `blrl`).
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
    let Some(call) = innermost else { return };
    let mut counter = s.record_counter.lock().expect("hle_watch counter");
    let record_no = *counter;
    *counter = counter.wrapping_add(1);
    drop(counter);
    let mut writer = s.writer.lock().expect("hle_watch writer mutex");
    let _ = writer.write_all(&[KIND_BODY_CALL]);
    let _ = writer.write_all(&record_no.to_le_bytes());
    let _ = writer.write_all(&call.nid.to_le_bytes());
    let _ = writer.write_all(&call.entry_record_no.to_le_bytes());
    let _ = writer.write_all(&pc.to_le_bytes());
    let _ = writer.write_all(&target.to_le_bytes());
    for r in &gpr[3..=10] {
        let _ = writer.write_all(&r.to_le_bytes());
    }
    let _ = writer.flush();
}

/// End-of-run `(entry_total, exit_total)`; `None` when inactive.
/// Lets the analyzer gate "zero records for NID X" on the hook
/// having fired at least once on some resolved NID.
pub fn totals() -> Option<(u64, u64)> {
    let s = state()?;
    let e = *s.entry_total.lock().expect("hle_watch entry_total");
    let x = *s.exit_total.lock().expect("hle_watch exit_total");
    Some((e, x))
}
