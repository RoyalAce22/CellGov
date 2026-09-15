//! The atomic commands name a line by any byte inside it.
//!
//! The bytes a `getllar` delivers, the reservation it takes, and the
//! range a `putllc` stores all cover the line containing the effective
//! address. Nothing refuses a misaligned address, so a guest that writes
//! one gets the containing line.

use crate::{SpuExecutionUnit, FAULT_MFC_READ_UNRESOLVED};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GETLLAR, MFC_PUTLLC};
use cellgov_sync::{ReservationTable, ReservedLine, RESERVATION_LINE_BYTES};
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// A second region past the one `GuestMemory::new` installs.
const AUX_BASE: u64 = 0x10000;
const AUX_LEN: u64 = 0x1000;
/// The one line of the auxiliary region the fixtures fill.
const LINE_EA: u64 = AUX_BASE + 0x80;
/// An address inside that line, 0x30 past its start.
const INSIDE_EA: u64 = LINE_EA + 0x30;
/// An address no region backs.
const UNMAPPED_EA: u64 = 0x9_0000;

const LINE: usize = RESERVATION_LINE_BYTES as usize;
const LSA: u32 = 0x200;

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> [u8; 4] {
    (0x081u32 << 23 | (imm << 7) | rt).to_be_bytes()
}

/// `wrch $chN, $rt`.
fn wrch(channel: u8, rt: u32) -> [u8; 4] {
    (0x10Du32 << 21 | (u32::from(channel) << 7) | rt).to_be_bytes()
}

/// The line's bytes, one value per offset, so local store says which
/// offset each byte came from.
fn counted_line() -> [u8; LINE] {
    let mut line = [0u8; LINE];
    for (i, b) in line.iter_mut().enumerate() {
        *b = i as u8;
    }
    line
}

/// A unit whose local store holds `il $10, cmd; wrch $ch21, $10` and
/// whose MFC channels name `ea`.
fn unit_issuing(cmd: u32, ea: u64) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    s.channels.mfc_lsa = LSA;
    s.channels.mfc_eah = (ea >> 32) as u32;
    s.channels.mfc_eal = ea as u32;
    s.channels.mfc_size = RESERVATION_LINE_BYTES as u32;
    s.channels.mfc_tag_id = 0;
    s.ls[0..4].copy_from_slice(&il(10, cmd));
    s.ls[4..8].copy_from_slice(&wrch(MFC_CMD, 10));
    unit
}

/// Guest memory whose auxiliary region holds [`counted_line`] at
/// [`LINE_EA`] and zeros elsewhere.
fn memory_with_counted_line() -> GuestMemory {
    let mut mem = GuestMemory::new(MEM_BYTES);
    mem.install_region(AUX_BASE, AUX_LEN as usize, "aux", PageSize::Page64K)
        .expect("the auxiliary region is clear of the base region");
    let range = ByteRange::new(GuestAddr::new(LINE_EA), RESERVATION_LINE_BYTES).expect("a line");
    mem.apply_commit(range, &counted_line())
        .expect("the auxiliary region is writable");
    mem
}

fn run_once(
    unit: &mut SpuExecutionUnit,
    ctx: &ExecutionContext<'_>,
    effects: &mut Vec<Effect>,
) -> cellgov_exec::ExecutionStepResult {
    unit.run_until_yield(Budget::new(100), ctx, effects)
}

fn acquired_lines(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::ReservationAcquire { line_addr, .. } => Some(*line_addr),
            _ => None,
        })
        .collect()
}

fn read_ranges(effects: &[Effect]) -> Vec<ByteRange> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some(*range),
            _ => None,
        })
        .collect()
}

#[test]
fn a_misaligned_getllar_delivers_and_reserves_the_containing_line() {
    let mem = memory_with_counted_line();
    let ctx = ExecutionContext::new(&mem);
    let mut unit = unit_issuing(MFC_GETLLAR, INSIDE_EA);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "{:?}",
        result.fault
    );
    let lsa = LSA as usize;
    assert_eq!(
        unit.state().ls[lsa..lsa + LINE],
        counted_line(),
        "local store holds the line from its first byte, not from the \
         address the guest wrote",
    );
    assert_eq!(
        unit.state().reservation.map(|l| l.addr()),
        Some(LINE_EA),
        "the reservation covers the same line",
    );
    assert_eq!(acquired_lines(&effects), vec![LINE_EA]);
    assert_eq!(
        read_ranges(&effects),
        vec![ByteRange::new(GuestAddr::new(LINE_EA), RESERVATION_LINE_BYTES).unwrap()],
        "and the read the dependency analysis sees is the line",
    );
}

#[test]
fn a_misaligned_getllar_near_the_region_end_reads_its_line_whole() {
    let mem = memory_with_counted_line();
    let ctx = ExecutionContext::new(&mem);
    let last_line = AUX_BASE + AUX_LEN - RESERVATION_LINE_BYTES;
    let ea = last_line + 0x40;
    assert!(
        mem.read(ByteRange::new(GuestAddr::new(ea), RESERVATION_LINE_BYTES).unwrap())
            .is_none(),
        "the premise: a line's worth of bytes from the raw address escapes \
         the region",
    );
    let mut unit = unit_issuing(MFC_GETLLAR, ea);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "the line the address falls inside resolves: {:?}",
        result.fault,
    );
    assert_eq!(unit.state().reservation.map(|l| l.addr()), Some(last_line));
}

#[test]
fn a_refused_getllar_names_the_line_not_the_byte() {
    let mem = memory_with_counted_line();
    let ctx = ExecutionContext::new(&mem);
    let mut unit = unit_issuing(MFC_GETLLAR, UNMAPPED_EA + 0x2C);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_READ_UNRESOLVED | (UNMAPPED_EA as u32 & 0xFFFF)
        )),
    );
}

#[test]
fn a_refused_getllar_leaves_the_atomic_status_alone() {
    let mem = memory_with_counted_line();
    let ctx = ExecutionContext::new(&mem);
    let mut unit = unit_issuing(MFC_GETLLAR, UNMAPPED_EA);
    // The model's "putllc lost its reservation".
    unit.state_mut().channels.atomic_status = 1;
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        unit.state().channels.atomic_status,
        1,
        "a command that did not complete reported no status",
    );
    assert!(unit.state().reservation.is_none());
}

/// G is bit 29 of the status word, so the channel reads `0x4`.
#[test]
fn a_completed_getllar_reports_the_getllar_bit() {
    let mem = memory_with_counted_line();
    let ctx = ExecutionContext::new(&mem);
    let mut unit = unit_issuing(MFC_GETLLAR, LINE_EA);
    unit.state_mut().channels.atomic_status = 1;
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_ne!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.state().channels.atomic_status, 0x4);
}

#[test]
fn a_misaligned_putllc_stores_over_the_reserved_line() {
    let mem = memory_with_counted_line();
    let mut table = ReservationTable::new();
    table.insert_or_replace(UnitId::new(UNIT), ReservedLine::containing(LINE_EA));
    let ctx = ExecutionContext::new(&mem).with_reservations(&table);
    let mut unit = unit_issuing(MFC_PUTLLC, INSIDE_EA);
    unit.state_mut().reservation = Some(ReservedLine::containing(LINE_EA));
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &ctx, &mut effects);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "{:?}",
        result.fault
    );
    let stores: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ConditionalStore { range, .. } => Some(*range),
            _ => None,
        })
        .collect();
    assert_eq!(
        stores,
        vec![ByteRange::new(GuestAddr::new(LINE_EA), RESERVATION_LINE_BYTES).unwrap()],
        "the store covers the line the reservation was held on",
    );
    assert_eq!(unit.state().channels.atomic_status, 0, "and succeeded");
}
