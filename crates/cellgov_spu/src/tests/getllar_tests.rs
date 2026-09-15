//! `getllar` reads its line or takes no reservation.
//!
//! The atomic path loads a line and acquires a reservation over it. The
//! reservation is the guest's evidence that it holds those bytes, so a
//! line that never arrived must leave no reservation behind: a later
//! `putllc` would otherwise succeed against a comparison the guest made
//! on stale local store.

use crate::{SpuExecutionUnit, FAULT_MFC_READ_UNRESOLVED};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_EAL, MFC_GETLLAR};
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// A second region, past the one `GuestMemory::new` installs, and an
/// address no region backs.
const AUX_BASE: u64 = 0x10000;
const AUX_LEN: usize = 0x1000;
const AUX_EA: u64 = AUX_BASE + 0x80;
const UNMAPPED_EA: u64 = 0x9_0000;

/// Another address no region backs, small enough for `il` to load it
/// whole as a positive immediate.
const UNMAPPED_NEAR_EA: u32 = 0x7F00;

/// `getllar` always moves a cache line.
const LINE_BYTES: u32 = 128;
const LSA: u32 = 0x200;

/// The byte filling the line, so local store says whether it arrived.
const MARK: u8 = 0x5C;

fn line_range(ea: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(ea), u64::from(LINE_BYTES)).expect("a 128-byte range")
}

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> [u8; 4] {
    (0x081u32 << 23 | (imm << 7) | rt).to_be_bytes()
}

/// `wrch $chN, $rt`.
fn wrch(channel: u8, rt: u32) -> [u8; 4] {
    (0x10Du32 << 21 | (u32::from(channel) << 7) | rt).to_be_bytes()
}

/// A unit whose local store holds `il $10, MFC_GETLLAR; wrch $ch21, $10`
/// and whose MFC channels name a line at `ea`.
fn unit_getllar(ea: u64) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    s.channels.mfc_lsa = LSA;
    s.channels.mfc_eah = (ea >> 32) as u32;
    s.channels.mfc_eal = ea as u32;
    s.channels.mfc_size = LINE_BYTES;
    s.channels.mfc_tag_id = 0;
    s.ls[0..4].copy_from_slice(&il(10, MFC_GETLLAR));
    s.ls[4..8].copy_from_slice(&wrch(MFC_CMD, 10));
    unit
}

/// [`unit_getllar`] at [`AUX_EA`], followed by a write to MFC_EAL and a
/// second `getllar` naming `second_ea`. Both run in one batch: the
/// atomic path returns to the fetch loop rather than yielding.
fn unit_getllar_twice(second_ea: u32) -> SpuExecutionUnit {
    let mut unit = unit_getllar(AUX_EA);
    let s = unit.state_mut();
    s.ls[8..12].copy_from_slice(&il(11, second_ea));
    s.ls[12..16].copy_from_slice(&wrch(MFC_EAL, 11));
    s.ls[16..20].copy_from_slice(&il(10, MFC_GETLLAR));
    s.ls[20..24].copy_from_slice(&wrch(MFC_CMD, 10));
    unit
}

/// Guest memory with a second region whose line is filled with
/// [`MARK`].
fn memory_with_marked_aux() -> GuestMemory {
    let mut mem = GuestMemory::new(MEM_BYTES);
    mem.install_region(AUX_BASE, AUX_LEN, "aux", PageSize::Page64K)
        .expect("the auxiliary region is clear of the base region");
    mem.apply_commit(line_range(AUX_EA), &[MARK; LINE_BYTES as usize])
        .expect("the auxiliary region is writable");
    mem
}

fn run_once(
    unit: &mut SpuExecutionUnit,
    mem: &GuestMemory,
    effects: &mut Vec<Effect>,
) -> cellgov_exec::ExecutionStepResult {
    let ctx = ExecutionContext::new(mem);
    unit.run_until_yield(Budget::new(100), &ctx, effects)
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

/// The premise: the auxiliary address resolves and sits past the base
/// region, so the old slice could not have reached it.
#[test]
fn the_auxiliary_line_resolves_and_sits_past_the_base_region() {
    let mem = memory_with_marked_aux();
    assert!(
        mem.read(line_range(AUX_EA)).is_some(),
        "the auxiliary line resolves",
    );
    assert!(
        mem.read(line_range(UNMAPPED_EA)).is_none(),
        "and the unmapped address does not",
    );
    assert!(
        AUX_EA as usize >= mem.as_bytes().len(),
        "the auxiliary line has to sit past the base region",
    );
}

/// A line in another region arrives, and its reservation is acquired.
#[test]
fn getllar_from_an_auxiliary_region_reads_its_line_and_acquires() {
    let mem = memory_with_marked_aux();
    let mut unit = unit_getllar(AUX_EA);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "the line resolves, so the read stands",
    );
    let lsa = LSA as usize;
    assert_eq!(
        &unit.state().ls[lsa..lsa + LINE_BYTES as usize],
        &[MARK; LINE_BYTES as usize],
        "local store holds the line the auxiliary region held",
    );
    assert_eq!(
        acquired_lines(&effects).len(),
        1,
        "and the reservation over the line it read is acquired",
    );
}

/// A line that resolves to nothing takes no reservation.
#[test]
fn getllar_from_an_unmapped_address_acquires_nothing_and_refuses() {
    let mem = memory_with_marked_aux();
    let mut unit = unit_getllar(UNMAPPED_EA);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    assert_eq!(
        result.yield_reason,
        YieldReason::Fault,
        "a line that never arrived is a refusal, not a held lock",
    );
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_READ_UNRESOLVED | (UNMAPPED_EA as u32 & 0xFFFF)
        )),
        "the refusal names itself and where it failed",
    );
    assert!(
        acquired_lines(&effects).is_empty(),
        "no reservation over a line the unit never read: {effects:?}",
    );
    assert!(
        unit.state().reservation.is_none(),
        "and the unit's own register holds none either, so a later \
         putllc has nothing to match",
    );
    assert_eq!(unit.status(), UnitStatus::Faulted, "the unit stops");
}

/// The destination boundary: the last [`LINE_BYTES`] of local store
/// hold the line exactly.
#[test]
fn a_getllar_ending_at_the_last_byte_of_local_store_lands() {
    let mem = memory_with_marked_aux();
    let mut unit = unit_getllar(AUX_EA);
    let lsa = unit.state().ls.len() - LINE_BYTES as usize;
    unit.state_mut().channels.mfc_lsa = lsa as u32;
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    assert_ne!(result.yield_reason, YieldReason::Fault, "the tail fits");
    assert_eq!(
        &unit.state().ls[lsa..],
        &[MARK; LINE_BYTES as usize],
        "the line filled local store to its last byte",
    );
    assert_eq!(
        acquired_lines(&effects).len(),
        1,
        "and the line it read is reserved",
    );
}

/// One byte further and the destination escapes the store, which is the
/// other arm the fault code names. The line itself resolves, so only the
/// local-store side refuses.
#[test]
fn a_getllar_whose_local_store_destination_escapes_refuses() {
    let mem = memory_with_marked_aux();
    let mut unit = unit_getllar(AUX_EA);
    let lsa = unit.state().ls.len() - LINE_BYTES as usize + 1;
    unit.state_mut().channels.mfc_lsa = lsa as u32;
    let before = unit.state().ls.clone();
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_READ_UNRESOLVED | (AUX_EA as u32 & 0xFFFF)
        )),
        "a destination the store cannot hold is refused, not truncated",
    );
    assert!(
        acquired_lines(&effects).is_empty(),
        "a line local store never received is not reserved: {effects:?}",
    );
    assert!(
        unit.state().reservation.is_none(),
        "and the unit's own register holds none either",
    );
    // Compared without dumping either side: local store is 256 KB.
    assert!(
        unit.state().ls == before,
        "the refused line left local store untouched",
    );
}

/// A refusal drops the reservation an earlier `getllar` in the same
/// batch took, in both halves of the state a `putllc` reads.
#[test]
fn a_refused_getllar_drops_a_reservation_the_same_batch_acquired() {
    let mem = memory_with_marked_aux();
    let mut unit = unit_getllar_twice(UNMAPPED_NEAR_EA);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    let lsa = LSA as usize;
    assert_eq!(
        &unit.state().ls[lsa..lsa + LINE_BYTES as usize],
        &[MARK; LINE_BYTES as usize],
        "the first getllar read its line, so there was a reservation to \
         drop",
    );
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_READ_UNRESOLVED | (UNMAPPED_NEAR_EA & 0xFFFF)
        )),
        "and the second one names the address that refused it",
    );
    assert!(
        effects.is_empty(),
        "the refused batch carries nothing, the first line's acquire \
         included: {effects:?}",
    );
    assert!(
        unit.state().reservation.is_none(),
        "and the register a putllc reads holds neither line",
    );
}

/// The detail bits cannot reach the class field.
///
/// The address is the guest's, so an unmasked detail would decode as
/// another fault class.
#[test]
fn a_refused_getllar_cannot_smear_into_the_fault_class() {
    let mem = memory_with_marked_aux();
    // Low half all ones, and no region backs it.
    let mut unit = unit_getllar(0x7_FFFF);
    let mut effects = Vec::new();
    let result = run_once(&mut unit, &mem, &mut effects);

    let Some(FaultKind::Guest(code)) = result.fault else {
        panic!("expected a guest fault, got {:?}", result.fault);
    };
    assert_eq!(
        code & 0xFFFF_0000,
        FAULT_MFC_READ_UNRESOLVED,
        "the class the reader decodes is the class that was raised: \
         0x{code:08x}",
    );
}
