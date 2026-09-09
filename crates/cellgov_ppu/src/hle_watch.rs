//! Env-gated per-call HLE return watch on guest PPU function entries.
//!
//! PRX load resolves watched NIDs to entry PCs through
//! [`register_nid_resolution`]; PPU dispatch calls [`on_dispatch`]
//! per instruction, and a per-thread stack of return PCs pairs each
//! exit with its entry.
//!
//! Caller contract: invoke [`is_active`] (or any state-touching API)
//! before the first dispatch so the [`OnceLock`] initializes and the
//! first watched instruction is not missed.
//!
//! Env vars:
//!
//!   CELLGOV_HLE_RETURN_WATCH       Comma-separated hex NIDs.
//!   CELLGOV_HLE_RETURN_WATCH_PCS   Comma-separated `pc=name` for
//!                                  entries whose NID is not unique
//!                                  across PRXes.
//!   CELLGOV_HLE_RETURN_WATCH_PATH  Output file path.
//!
//! File format (little-endian, no padding inside records): a "CGHW"
//! version-1 header carrying the watched-ID directory (raw-PC
//! synthetic IDs are `pc | 0x80000000`), then the records `wire`
//! builds, one builder per record kind.

#![allow(clippy::print_stderr)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

/// Record layouts: one builder per kind returns the complete record,
/// kind byte first, every field little-endian, no padding. A layout
/// change is a change here and in the reader.
mod wire {
    pub(super) const KIND_ENTRY: u8 = 1;
    pub(super) const KIND_EXIT: u8 = 2;
    pub(super) const KIND_RESOLUTION: u8 = 3;
    /// Body-event `sc` inside a watched function.
    pub(super) const KIND_BODY_SYSCALL: u8 = 4;
    /// Return point of a body-event `sc`, paired to its entry record.
    pub(super) const KIND_BODY_SYSCALL_RETURN: u8 = 5;
    /// Body-event `bl` / `bctrl` / `blrl` inside a watched function.
    pub(super) const KIND_BODY_CALL: u8 = 6;

    /// r3..r10 as eight u64s.
    const ARGS_LEN: usize = 8 * 8;
    pub(super) const ENTRY_LEN: usize = 1 + 8 + 4 + 4 + 4 + 4 + ARGS_LEN;
    pub(super) const EXIT_LEN: usize = 1 + 8 + 4 + 8 + 4 + 8;
    pub(super) const BODY_SYSCALL_LEN: usize = 1 + 8 + 4 + 8 + 4 + 4 + ARGS_LEN;
    pub(super) const BODY_SYSCALL_RETURN_LEN: usize = 1 + 8 + 4 + 8 + 4 + 4 + 8;
    pub(super) const BODY_CALL_LEN: usize = 1 + 8 + 4 + 8 + 4 + 4 + ARGS_LEN;
    /// Fixed part of a resolution record; the name follows, at most
    /// 255 bytes behind its 1-byte length.
    pub(super) const RESOLUTION_HEAD_LEN: usize = 1 + 4 + 4 + 1;

    struct Rec(Vec<u8>);

    impl Rec {
        fn new(kind: u8, len: usize) -> Self {
            let mut v = Vec::with_capacity(len);
            v.push(kind);
            Rec(v)
        }
        fn u8(mut self, v: u8) -> Self {
            self.0.push(v);
            self
        }
        fn u32(mut self, v: u32) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u64(mut self, v: u64) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn args(mut self, gpr: &[u64; 32]) -> Self {
            for r in &gpr[3..=10] {
                self.0.extend_from_slice(&r.to_le_bytes());
            }
            self
        }
        fn finish(self, len: usize) -> Vec<u8> {
            debug_assert_eq!(self.0.len(), len, "record kind {}", self.0[0]);
            self.0
        }
    }

    /// Watched ID resolved to an entry PC; carries no record number.
    pub(super) fn resolution(on_wire_nid: u32, entry_pc: u32, name: &str) -> Vec<u8> {
        let name = &name.as_bytes()[..name.len().min(255)];
        let mut rec = Rec::new(KIND_RESOLUTION, RESOLUTION_HEAD_LEN + name.len())
            .u32(on_wire_nid)
            .u32(entry_pc)
            .u8(name.len() as u8)
            .0;
        rec.extend_from_slice(name);
        rec
    }

    pub(super) fn entry(
        record_no: u64,
        on_wire_nid: u32,
        entry_pc: u32,
        pc: u32,
        lr: u32,
        gpr: &[u64; 32],
    ) -> Vec<u8> {
        Rec::new(KIND_ENTRY, ENTRY_LEN)
            .u64(record_no)
            .u32(on_wire_nid)
            .u32(entry_pc)
            .u32(pc)
            .u32(lr)
            .args(gpr)
            .finish(ENTRY_LEN)
    }

    pub(super) fn exit(
        record_no: u64,
        on_wire_nid: u32,
        entry_record_no: u64,
        pc: u32,
        r3: u64,
    ) -> Vec<u8> {
        Rec::new(KIND_EXIT, EXIT_LEN)
            .u64(record_no)
            .u32(on_wire_nid)
            .u64(entry_record_no)
            .u32(pc)
            .u64(r3)
            .finish(EXIT_LEN)
    }

    pub(super) fn body_syscall(
        record_no: u64,
        on_wire_nid: u32,
        entry_record_no: u64,
        syscall_num: u32,
        pc: u32,
        gpr: &[u64; 32],
    ) -> Vec<u8> {
        Rec::new(KIND_BODY_SYSCALL, BODY_SYSCALL_LEN)
            .u64(record_no)
            .u32(on_wire_nid)
            .u64(entry_record_no)
            .u32(syscall_num)
            .u32(pc)
            .args(gpr)
            .finish(BODY_SYSCALL_LEN)
    }

    pub(super) fn body_syscall_return(
        record_no: u64,
        on_wire_nid: u32,
        entry_record_no: u64,
        syscall_num: u32,
        pc: u32,
        r3: u64,
    ) -> Vec<u8> {
        Rec::new(KIND_BODY_SYSCALL_RETURN, BODY_SYSCALL_RETURN_LEN)
            .u64(record_no)
            .u32(on_wire_nid)
            .u64(entry_record_no)
            .u32(syscall_num)
            .u32(pc)
            .u64(r3)
            .finish(BODY_SYSCALL_RETURN_LEN)
    }

    pub(super) fn body_call(
        record_no: u64,
        on_wire_nid: u32,
        entry_record_no: u64,
        pc: u32,
        target: u32,
        gpr: &[u64; 32],
    ) -> Vec<u8> {
        Rec::new(KIND_BODY_CALL, BODY_CALL_LEN)
            .u64(record_no)
            .u32(on_wire_nid)
            .u64(entry_record_no)
            .u32(pc)
            .u32(target)
            .args(gpr)
            .finish(BODY_CALL_LEN)
    }
}

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

impl WriterState {
    /// Assign the next record number, build the record with it, and
    /// append it; returns the number so a later record can pair to it.
    fn append(&mut self, build: impl FnOnce(u64) -> Vec<u8>) -> u64 {
        let record_no = self.record_counter;
        self.record_counter = self.record_counter.wrapping_add(1);
        let _ = self.writer.write_all(&build(record_no));
        let _ = self.writer.flush();
        record_no
    }
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
        if let Err(e) = writer.write_all(&wire::resolution(on_wire, *pc, name)) {
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
/// Raw-PC watches are pre-resolved at init and are not included
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
    let _ = w.writer.write_all(&wire::resolution(nid, entry_pc, name));
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
        w.exit_total = w.exit_total.wrapping_add(1);
        w.append(|record_no| {
            wire::exit(
                record_no,
                call.on_wire_nid,
                call.entry_record_no,
                pc,
                gpr[3],
            )
        });
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
        w.entry_total = w.entry_total.wrapping_add(1);
        let record_no =
            w.append(|record_no| wire::entry(record_no, on_wire_nid, entry_pc, pc, lr32, gpr));
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
        w.append(|record_no| {
            wire::body_syscall_return(
                record_no,
                pending.in_flight_on_wire_nid,
                pending.entry_record_no,
                pending.syscall_num,
                pc,
                gpr[3],
            )
        });
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
    w.append(|record_no| {
        wire::body_syscall(
            record_no,
            call.on_wire_nid,
            call.entry_record_no,
            syscall_num,
            pc,
            gpr,
        )
    });
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
    w.append(|record_no| {
        wire::body_call(
            record_no,
            call.on_wire_nid,
            call.entry_record_no,
            pc,
            target,
            gpr,
        )
    });
}

/// End-of-run `(entry_total, exit_total, dropped_body_events)`; `None`
/// when inactive.
pub fn totals() -> Option<(u64, u64, u64)> {
    let s = state()?;
    let w = s.writer.lock().expect("hle_watch writer");
    Some((w.entry_total, w.exit_total, w.dropped_body_events))
}

#[cfg(test)]
#[path = "tests/hle_watch_wire_tests.rs"]
mod wire_tests;
