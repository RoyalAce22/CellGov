//! The HLE return watch: entry, exit and body events of chosen guest
//! functions, named by NID or, for an entry whose NID is not unique
//! across PRXes, by raw entry PC.
//!
//! File format (little-endian, no padding inside records): a "CGHW"
//! version-1 header carrying the watched-ID directory (raw-PC
//! synthetic IDs are `pc | 0x80000000`), then the records `wire`
//! builds, one builder per record kind.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;

use cellgov_event::UnitId;
use cellgov_ppu::instruction::{branch_target, PpuInstruction};
use cellgov_ppu::state::PpuState;

use super::record_file::{FirstFailure, RecordFile};

/// Record layouts: one builder per kind returns the complete record,
/// kind byte first, every field little-endian, no padding. A layout
/// change is a change here and in the reader.
pub(crate) mod wire {
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

/// The bit that marks a raw-PC watch's synthetic on-wire ID.
pub const RAW_PC_ID_BIT: u32 = 0x8000_0000;

/// The longest name a resolution record carries, behind its 1-byte
/// length.
pub const MAX_NAME_LEN: usize = u8::MAX as usize;

/// What the watch records and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleWatchSpec {
    /// Real NIDs, resolved to entry PCs when a firmware set binds.
    pub nids: Vec<u32>,
    /// Entry PCs watched as given, with the name to report them under.
    pub raw_pcs: Vec<(u32, String)>,
    /// Where the capture goes.
    pub path: PathBuf,
}

impl HleWatchSpec {
    /// The header's watched-ID directory: real NIDs first, then the
    /// raw-PC synthetic IDs.
    fn on_wire_ids(&self) -> Vec<u32> {
        self.nids
            .iter()
            .copied()
            .chain(self.raw_pcs.iter().map(|(pc, _)| pc | RAW_PC_ID_BIT))
            .collect()
    }

    /// The whole file header.
    #[must_use]
    pub fn header(&self) -> Vec<u8> {
        let ids = self.on_wire_ids();
        let mut header = Vec::with_capacity(12 + 4 * ids.len());
        header.extend_from_slice(b"CGHW");
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&(ids.len() as u32).to_le_bytes());
        for id in &ids {
            header.extend_from_slice(&id.to_le_bytes());
        }
        header
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum WatchKey {
    Nid(u32),
    RawPc(u32),
}

#[derive(Clone, Copy)]
struct Resolved {
    on_wire_nid: u32,
    entry_pc: u32,
}

#[derive(Clone, Copy)]
struct InFlightCall {
    unit: UnitId,
    on_wire_nid: u32,
    return_pc: u32,
    entry_record_no: u64,
}

#[derive(Clone, Copy)]
struct PendingSyscallReturn {
    unit: UnitId,
    return_pc: u32,
    syscall_num: u32,
    on_wire_nid: u32,
    entry_record_no: u64,
}

/// The watch's state across every PPU unit: one call stack of watched
/// functions, one record counter.
pub struct HleWatch<W: Write> {
    watched_nids: Vec<u32>,
    resolved: BTreeMap<WatchKey, Resolved>,
    out: RecordFile<W>,
    record_counter: u64,
    in_flight: Vec<InFlightCall>,
    pending_syscall_returns: Vec<PendingSyscallReturn>,
    last_dispatch: BTreeMap<UnitId, (u32, u32)>,
    failure: FirstFailure,
}

impl<W: Write> HleWatch<W> {
    /// A watch writing to `out`, whose header is already written, with
    /// the raw-PC watches resolved.
    ///
    /// A resolution record that `out` refuses stays held for
    /// [`Self::take_write_failure`].
    pub fn new(spec: &HleWatchSpec, out: RecordFile<W>) -> Self {
        let mut watch = Self {
            watched_nids: spec.nids.clone(),
            resolved: BTreeMap::new(),
            out,
            record_counter: 0,
            in_flight: Vec::new(),
            pending_syscall_returns: Vec::new(),
            last_dispatch: BTreeMap::new(),
            failure: FirstFailure::default(),
        };
        for (pc, name) in &spec.raw_pcs {
            let on_wire_nid = pc | RAW_PC_ID_BIT;
            watch.resolved.insert(
                WatchKey::RawPc(*pc),
                Resolved {
                    on_wire_nid,
                    entry_pc: *pc,
                },
            );
            watch
                .failure
                .note(watch.out.append(&wire::resolution(on_wire_nid, *pc, name)));
        }
        watch
    }

    /// Resolve `nid` to `entry_pc` and write its resolution record; a
    /// NID already resolved keeps its first entry PC.
    fn resolve_nid(&mut self, nid: u32, name: &str, entry_pc: u32) {
        let key = WatchKey::Nid(nid);
        if !self.watched_nids.contains(&nid) || self.resolved.contains_key(&key) {
            return;
        }
        self.resolved.insert(
            key,
            Resolved {
                on_wire_nid: nid,
                entry_pc,
            },
        );
        self.failure
            .note(self.out.append(&wire::resolution(nid, entry_pc, name)));
    }

    fn append(&mut self, build: impl FnOnce(u64) -> Vec<u8>) -> u64 {
        let record_no = self.record_counter;
        self.record_counter = self.record_counter.wrapping_add(1);
        self.failure.note(self.out.append(&build(record_no)));
        record_no
    }

    /// Record what `insn`, about to execute in `state`, means for the
    /// watched functions.
    pub fn dispatch(&mut self, unit: UnitId, insn: &PpuInstruction, state: &PpuState) {
        let pc = state.pc as u32;
        let gpr = state.gpr.as_array();
        self.on_dispatch(unit, pc, gpr, state.lr());
        match *insn {
            PpuInstruction::Sc { .. } => self.on_syscall(unit, pc, gpr),
            PpuInstruction::B {
                offset,
                aa,
                link: true,
            } => {
                // The on-wire format carries 32-bit PCs.
                let target = branch_target(u64::from(pc), offset, aa) as u32;
                self.on_branch_link(unit, pc, gpr, target);
            }
            PpuInstruction::Bcctr { link: true, .. } => {
                self.on_branch_link(unit, pc, gpr, state.ctr() as u32);
            }
            PpuInstruction::Bclr { link: true, .. } => {
                self.on_branch_link(unit, pc, gpr, state.lr() as u32);
            }
            _ => {}
        }
    }

    fn on_dispatch(&mut self, unit: UnitId, pc: u32, gpr: &[u64; 32], lr: u64) {
        // Exit before entry, so a PC that is both an outer frame's
        // return PC and a watched entry pops the outer frame first.
        //
        // PC-equality limitation: when `lr == entry_pc`, the return
        // visit pops then matches entry at the same PC, emitting a
        // phantom entry. Analyzers should treat back-to-back entries
        // with no body events as suspect on PCs that host this pattern.
        if let Some(index) = self
            .in_flight
            .iter()
            .rposition(|c| c.unit == unit && c.return_pc == pc)
        {
            let call = self.in_flight[index];
            self.append(|record_no| {
                wire::exit(
                    record_no,
                    call.on_wire_nid,
                    call.entry_record_no,
                    pc,
                    gpr[3],
                )
            });
            self.in_flight.remove(index);
        }

        let entry = self.resolved.values().find(|r| r.entry_pc == pc).copied();
        if let Some(r) = entry {
            // The on-wire format carries 32-bit PCs; a PS3 guest
            // address fits, a 64-bit LR with high bits does not.
            debug_assert!(
                lr >> 32 == 0,
                "PPU lr 0x{lr:016x} exceeds 32-bit; the on-wire format is u32"
            );
            let lr32 = lr as u32;
            // A unit reports an instruction again when a full store
            // buffer sends it back (the `PpuTap::dispatch` contract), and
            // a watched function's first instruction is often a store.
            // A match on the same frame, with nothing recorded since its
            // entry, is that retry.
            let retried = self
                .last_dispatch
                .get(&unit)
                .is_some_and(|&(last_pc, last_lr)| last_pc == pc && last_lr == lr32);
            if !retried {
                let record_no = self.append(|record_no| {
                    wire::entry(record_no, r.on_wire_nid, r.entry_pc, pc, lr32, gpr)
                });
                self.in_flight.push(InFlightCall {
                    unit,
                    on_wire_nid: r.on_wire_nid,
                    return_pc: lr32,
                    entry_record_no: record_no,
                });
            }
        }

        if let Some(index) = self
            .pending_syscall_returns
            .iter()
            .rposition(|p| p.unit == unit && p.return_pc == pc)
        {
            let p = self.pending_syscall_returns[index];
            self.append(|record_no| {
                wire::body_syscall_return(
                    record_no,
                    p.on_wire_nid,
                    p.entry_record_no,
                    p.syscall_num,
                    pc,
                    gpr[3],
                )
            });
            self.pending_syscall_returns.remove(index);
        }
        self.last_dispatch.insert(unit, (pc, lr as u32));
    }

    /// Record an `sc` during a watched call and queue its return at `pc + 4`.
    ///
    /// The watch ignores an `sc` while no watched call is open.
    fn on_syscall(&mut self, unit: UnitId, pc: u32, gpr: &[u64; 32]) {
        let Some(call) = self
            .in_flight
            .iter()
            .rev()
            .find(|c| c.unit == unit)
            .copied()
        else {
            return;
        };
        let syscall_num = gpr[11] as u32;
        self.append(|record_no| {
            wire::body_syscall(
                record_no,
                call.on_wire_nid,
                call.entry_record_no,
                syscall_num,
                pc,
                gpr,
            )
        });
        self.pending_syscall_returns.push(PendingSyscallReturn {
            unit,
            return_pc: pc.wrapping_add(4),
            syscall_num,
            on_wire_nid: call.on_wire_nid,
            entry_record_no: call.entry_record_no,
        });
    }

    /// Record a `bl` / `bctrl` / `blrl` during a watched call.
    ///
    /// The watch ignores such a branch while no watched call is open.
    fn on_branch_link(&mut self, unit: UnitId, pc: u32, gpr: &[u64; 32], target: u32) {
        let Some(call) = self
            .in_flight
            .iter()
            .rev()
            .find(|c| c.unit == unit)
            .copied()
        else {
            return;
        };
        self.append(|record_no| {
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

    /// The first write failure since the last call, once. The capture
    /// ends at it; the watch keeps tracking calls and writes nothing.
    pub fn take_write_failure(&mut self) -> Option<std::io::Error> {
        self.failure.take()
    }

    /// The writer the records went to.
    #[cfg(test)]
    pub(crate) fn into_inner(self) -> W {
        self.out.into_inner()
    }
}

impl<W: Write> HleWatch<W> {
    /// Resolve the watched NIDs against a bound firmware set, and return the lines to report.
    ///
    /// `exports` maps library -> NID -> OPD address, and `opd_code`
    /// reads the code address that an OPD holds. A NID that several
    /// libraries export names no single function, so the watch leaves
    /// it unresolved. A NID that an earlier set resolved keeps that
    /// entry PC. When this set's entry for the NID is at a different
    /// address, a line names it, because the watch records no call to it.
    pub fn bind(
        &mut self,
        exports: &BTreeMap<String, BTreeMap<u32, u32>>,
        opd_code: impl Fn(u32) -> Option<u32>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut tried = BTreeSet::new();
        for nid in self.watched_nids.clone() {
            if !tried.insert(nid) {
                continue;
            }
            let hits: Vec<(&str, u32)> = exports
                .iter()
                .filter_map(|(ns, by_nid)| by_nid.get(&nid).map(|&opd| (ns.as_str(), opd)))
                .collect();
            if let Some(first) = self.resolved.get(&WatchKey::Nid(nid)).map(|r| r.entry_pc) {
                let moved = match hits.as_slice() {
                    [(_, opd)] => opd_code(*opd).filter(|&pc| pc != first),
                    _ => None,
                };
                if let Some(entry_pc) = moved {
                    lines.push(format!(
                        "NID 0x{nid:08x} stays watched at 0x{first:08x}, where an \
                         earlier firmware set bound it; this set's entry \
                         0x{entry_pc:08x} is not watched"
                    ));
                }
                continue;
            }
            let (library, opd) = match hits.as_slice() {
                [] => {
                    lines.push(format!(
                        "NID 0x{nid:08x} not present in firmware export table"
                    ));
                    continue;
                }
                [one] => *one,
                several => {
                    let libs: Vec<&str> = several.iter().map(|(ns, _)| *ns).collect();
                    lines.push(format!(
                        "NID 0x{nid:08x} is exported by {} libraries ({}); it names no \
                         single function, so it is not watched",
                        libs.len(),
                        libs.join(", ")
                    ));
                    continue;
                }
            };
            let Some(entry_pc) = opd_code(opd) else {
                lines.push(format!("NID 0x{nid:08x} OPD at 0x{opd:08x} not mapped"));
                continue;
            };
            let name = cellgov_ps3_abi::nid::lookup(nid)
                .map(|(_, fname)| fname)
                .unwrap_or(library);
            self.resolve_nid(nid, name, entry_pc);
            lines.push(format!(
                "resolved NID 0x{nid:08x} ({name}) entry_pc=0x{entry_pc:08x}"
            ));
        }
        lines
    }
}

#[cfg(test)]
#[path = "tests/hle_watch_tests.rs"]
mod tests;
