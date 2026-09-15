//! A parked MFC GET moves its bytes or names its refusal.
//!
//! The transfer resolves its effective address the way guest memory
//! resolves any address, so it reaches a region other than the one at
//! the base. Where it resolves to nothing there are no bytes to move,
//! and the tag bit stays clear: it is the guest's only signal that the
//! transfer finished.

use crate::{SpuExecutionUnit, FAULT_MFC_GET_UNRESOLVED};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::{GuestMemory, PageSize};
use cellgov_ps3_abi::hw::spu::MFC_GET;
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// A second region, clear of the one `GuestMemory::new` installs at the
/// base. An address here resolves, and a slice of the base region's
/// bytes does not reach it.
const AUX_BASE: u64 = 0x10000;
const AUX_LEN: usize = 0x1000;

/// Inside the auxiliary region, and past the end of every region.
const AUX_EA: u64 = AUX_BASE + 0x40;
const UNMAPPED_EA: u64 = 0x9_0000;

const TRANSFER_BYTES: u32 = 64;
const LSA: u32 = 0x200;
const TAG: u8 = 3;

/// The byte the transfer's source is filled with, so local store says
/// whether the transfer read from there.
const MARK: u8 = 0xA7;

/// A unit whose local store holds `il $10, MFC_GET; wrch $ch21, $10`
/// and whose MFC channels name a transfer from `ea`.
fn unit_getting(ea: u64) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    s.channels.mfc_lsa = LSA;
    s.channels.mfc_eah = (ea >> 32) as u32;
    s.channels.mfc_eal = ea as u32;
    s.channels.mfc_size = TRANSFER_BYTES;
    s.channels.mfc_tag_id = u32::from(TAG);
    let il_raw: u32 = 0x081 << 23 | (MFC_GET << 7) | 10;
    s.ls[0..4].copy_from_slice(&il_raw.to_be_bytes());
    let wrch_raw: u32 = 0x10D << 21 | (21u32 << 7) | 10;
    s.ls[4..8].copy_from_slice(&wrch_raw.to_be_bytes());
    unit
}

/// Guest memory with a second region, zero-filled.
fn memory_with_aux_region() -> GuestMemory {
    let mut mem = GuestMemory::new(MEM_BYTES);
    mem.install_region(AUX_BASE, AUX_LEN, "aux", PageSize::Page64K)
        .expect("the auxiliary region is clear of the base region");
    mem
}

/// The transfer's range, for the source checks and for filling it.
fn transfer_range(ea: u64) -> cellgov_mem::ByteRange {
    cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ea), u64::from(TRANSFER_BYTES))
        .expect("a 64-byte range")
}

/// [`memory_with_aux_region`] with the transfer's source bytes set to
/// [`MARK`], so local store says whether it read from there.
fn memory_with_marked_source() -> GuestMemory {
    let mut mem = memory_with_aux_region();
    mem.apply_commit(transfer_range(AUX_EA), &[MARK; TRANSFER_BYTES as usize])
        .expect("the auxiliary region is writable");
    mem
}

/// Runs the issuing step, then the step that performs the parked
/// transfer, and returns the second step's result.
fn issue_then_perform(
    unit: &mut SpuExecutionUnit,
    mem: &GuestMemory,
) -> cellgov_exec::ExecutionStepResult {
    let ctx = ExecutionContext::new(mem);
    let mut effects = Vec::new();
    let issuing = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(
        issuing.yield_reason,
        YieldReason::DmaSubmitted,
        "the first step has to park the transfer for the second to perform it",
    );
    unit.run_until_yield(Budget::new(100), &ctx, &mut effects)
}

/// The premise: the two addresses resolve differently, and only one of
/// them lies in the region `GuestMemory::new` installs.
#[test]
fn the_auxiliary_region_resolves_and_the_unmapped_address_does_not() {
    let mem = memory_with_aux_region();
    assert!(
        mem.read(transfer_range(AUX_EA)).is_some(),
        "the auxiliary region resolves"
    );
    assert!(
        mem.read(transfer_range(UNMAPPED_EA)).is_none(),
        "and the unmapped address does not"
    );
    assert!(
        AUX_EA as usize >= mem.as_bytes().len(),
        "the auxiliary address also has to sit past the base region, or \
         a slice of it would reach the transfer's source too",
    );
}

/// A transfer out of a region other than the base one delivers its
/// bytes.
#[test]
fn a_get_from_an_auxiliary_region_reaches_local_store() {
    let mem = memory_with_marked_source();
    let mut unit = unit_getting(AUX_EA);
    let result = issue_then_perform(&mut unit, &mem);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "the source resolves, so the transfer has bytes to move",
    );
    let lsa = LSA as usize;
    assert_eq!(
        &unit.state().ls[lsa..lsa + TRANSFER_BYTES as usize],
        &[MARK; TRANSFER_BYTES as usize],
        "local store holds what the auxiliary region held",
    );
    assert_eq!(
        unit.state().channels.tag_status & (1u32 << TAG),
        1u32 << TAG,
        "and the tag bit reports the completion it earned",
    );
}

/// A transfer whose source resolves to nothing publishes no tag bit.
#[test]
fn a_get_from_an_unmapped_address_faults_and_publishes_no_tag_bit() {
    let mem = memory_with_aux_region();
    let mut unit = unit_getting(UNMAPPED_EA);
    let result = issue_then_perform(&mut unit, &mem);

    assert_eq!(
        result.yield_reason,
        YieldReason::Fault,
        "a transfer that moved nothing is a refusal, not a completion",
    );
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_MFC_GET_UNRESOLVED | u32::from(TAG))),
        "the refusal names itself and the tag it was issued under",
    );
    assert_eq!(
        unit.state().channels.tag_status & (1u32 << TAG),
        0,
        "the tag bit is the guest's only completion signal, and nothing \
         completed",
    );
    assert_eq!(
        unit.status(),
        UnitStatus::Faulted,
        "the unit does not run on past a transfer it never received",
    );
}

/// A zero-byte transfer is a size the architecture allows. It reads no
/// main storage, so the address it names does not have to resolve.
#[test]
fn a_zero_byte_get_completes_where_no_region_backs_its_address() {
    let mem = memory_with_aux_region();
    let mut unit = unit_getting(UNMAPPED_EA);
    unit.state_mut().channels.mfc_size = 0;
    let result = issue_then_perform(&mut unit, &mem);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "no bytes were asked for, so nothing failed to arrive",
    );
    assert_eq!(
        unit.state().channels.tag_status & (1u32 << TAG),
        1u32 << TAG,
        "a transfer that had nothing to move still finished",
    );
}

/// The destination boundary: the last [`TRANSFER_BYTES`] of local store
/// hold the transfer exactly.
#[test]
fn a_get_ending_at_the_last_byte_of_local_store_lands() {
    let mem = memory_with_marked_source();
    let mut unit = unit_getting(AUX_EA);
    let lsa = unit.state().ls.len() - TRANSFER_BYTES as usize;
    unit.state_mut().channels.mfc_lsa = lsa as u32;
    let result = issue_then_perform(&mut unit, &mem);

    assert_ne!(result.yield_reason, YieldReason::Fault, "the tail fits");
    assert_eq!(
        &unit.state().ls[lsa..],
        &[MARK; TRANSFER_BYTES as usize],
        "the transfer filled local store to its last byte",
    );
}

/// One byte further and the destination escapes the store, which is the
/// other half of the refusal the fault code names.
#[test]
fn a_get_whose_local_store_destination_escapes_faults() {
    let mem = memory_with_marked_source();
    let mut unit = unit_getting(AUX_EA);
    let lsa = unit.state().ls.len() - TRANSFER_BYTES as usize + 1;
    unit.state_mut().channels.mfc_lsa = lsa as u32;
    let before = unit.state().ls.clone();
    let result = issue_then_perform(&mut unit, &mem);

    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_MFC_GET_UNRESOLVED | u32::from(TAG))),
        "a destination the store cannot hold is refused, not truncated",
    );
    assert_eq!(
        unit.state().channels.tag_status & (1u32 << TAG),
        0,
        "and no tag bit reports a transfer local store never received",
    );
    // Compared without dumping either side: local store is 256 KB.
    assert!(
        unit.state().ls == before,
        "the refused transfer left local store untouched",
    );
}
