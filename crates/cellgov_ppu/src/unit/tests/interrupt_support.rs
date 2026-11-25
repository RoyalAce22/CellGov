//! The memory, the kernel thread and the instruction words the interrupt
//! delivery tests share.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::PpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_ME, MSR_RI, MSR_SF, SLB_VALID};
use cellgov_time::{Budget, GuestTicks};

/// The memory size: 4 MB, which holds the 4.93 system call handler's real
/// address.
pub(super) const MEMORY: usize = 0x40_0000;
/// The page table: 2^11 groups at real 1 MB.
pub(super) const HTAB: u64 = 0x10_0000;
/// Where each test's code starts, at a real address.
pub(super) const CODE: u64 = 0x2_0000;
/// Data the real-mode tests read and write.
pub(super) const DATA: u64 = 0x3_0000;
/// The segment SLB entry 0 maps: EA 0xC000_0000, VSID 0x123.
pub(super) const SEGMENT: u64 = 0xC000_0000;
pub(super) const VSID: u64 = 0x123;
/// A segment no SLB entry maps.
pub(super) const NO_SEGMENT: u64 = 0xD000_0000;

/// A supervisor MSR with bits that interrupt entry clears and keeps:
/// 64-bit, machine checks on, floating point on, recoverable.
pub(super) const KERNEL_MSR: u64 = MSR_SF | MSR_ME | MSR_FP | MSR_RI;

/// `nop` (`ori r0,r0,0`).
pub(super) const NOP: u32 = 0x6000_0000;
/// `tw 31,0,0`: a trap whose condition always holds.
pub(super) const TRAP: u32 = 0x7FE0_0008;
/// `sc` with LEV = 0.
pub(super) const SC: u32 = 0x4400_0002;
/// `sc` with LEV = 1.
pub(super) const SC_1: u32 = 0x4400_0022;
/// `rfid`.
pub(super) const RFID: u32 = 0x4C00_0024;
/// The word 0, which decodes to no instruction.
pub(super) const ILLEGAL: u32 = 0;

fn d_form(op: u32, rt: u32, ra: u32, d: u16) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn x_form(rt: u32, ra: u32, rb: u32, xo: u32) -> u32 {
    (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (xo << 1)
}

/// `addi rt,ra,si`.
pub(super) fn addi(rt: u32, ra: u32, si: u16) -> u32 {
    d_form(14, rt, ra, si)
}

/// `lwz rt,d(ra)`.
pub(super) fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    d_form(32, rt, ra, d)
}

/// `stw rs,d(ra)`.
pub(super) fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    d_form(36, rs, ra, d)
}

/// `ld rt,ds(ra)`.
pub(super) fn ld(rt: u32, ds: u16, ra: u32) -> u32 {
    d_form(58, rt, ra, ds & 0xFFFC)
}

/// `lwarx rt,ra,rb`.
pub(super) fn lwarx(rt: u32, ra: u32, rb: u32) -> u32 {
    x_form(rt, ra, rb, 20)
}

/// `mfmsr rt`.
pub(super) fn mfmsr(rt: u32) -> u32 {
    x_form(rt, 0, 0, 83)
}

/// `mtmsrd rs,l`.
pub(super) fn mtmsrd(rs: u32, l: bool) -> u32 {
    x_form(rs, u32::from(l), 0, 178)
}

/// `mfspr rt,spr`.
pub(super) fn mfspr(rt: u32, spr: u16) -> u32 {
    let spr = u32::from(spr);
    x_form(rt, spr & 0x1F, spr >> 5, 339)
}

/// `mtspr spr,rs`.
pub(super) fn mtspr(spr: u16, rs: u32) -> u32 {
    let spr = u32::from(spr);
    x_form(rs, spr & 0x1F, spr >> 5, 467)
}

/// Writes `bytes` at real address `at`.
pub(super) fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("a range");
    mem.apply_commit(range, bytes)
        .expect("the memory holds the range");
}

/// Writes `words` from real address `at`.
pub(super) fn put_words(mem: &mut GuestMemory, at: u64, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        put(mem, at + 4 * i as u64, &w.to_be_bytes());
    }
}

/// [`MEMORY`] bytes with `code` at [`CODE`].
pub(super) fn memory(code: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::new(MEMORY);
    put_words(&mut mem, CODE, code);
    mem
}

/// Writes the page table entry for page `index` of [`SEGMENT`] to slot 0
/// of its primary group, at real address `ra` with protection bits `pp`.
pub(super) fn map_page(mem: &mut GuestMemory, index: u64, ra: u64, pp: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | pp).to_be_bytes());
}

/// A thread at [`CODE`] with MSR `msr`, the page table at [`HTAB`], real
/// mode over every effective address, and SLB entry 0 over [`SEGMENT`].
pub(super) fn kernel_unit(msr: u64) -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(msr);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition =
        Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).expect("a valid partition");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = CODE;
    unit
}

/// The thread's supervisor registers.
pub(super) fn sup(unit: &PpuExecutionUnit) -> &SupervisorState {
    unit.state().supervisor().expect("supervisor registers")
}

/// Runs one batch of at most `budget` instructions at guest tick 0.
pub(super) fn run(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
) -> (ExecutionStepResult, Vec<Effect>) {
    run_at(unit, mem, budget, 0)
}

/// Runs one batch of at most `budget` instructions at guest tick `tick`.
pub(super) fn run_at(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
    tick: u64,
) -> (ExecutionStepResult, Vec<Effect>) {
    let mut effects = Vec::new();
    let ctx = ExecutionContext::new(mem).with_current_tick(GuestTicks::new(tick));
    let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    (r, effects)
}
