//! The read intents the SPU transfer paths record.

use crate::SpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{MFC_GET, MFC_GETLLAR};
use cellgov_time::Budget;

const UNIT: u64 = 7;
/// An address inside a line. A `get` reads from it as written; a
/// `getllar` reads the line containing it.
const SOURCE_EA: u64 = 0x1040;
const SOURCE_LINE_EA: u64 = 0x1000;
const TRANSFER_BYTES: u32 = 128;
const MEM_BYTES: usize = 0x2000;
const LAST_LINE_EA: u64 = MEM_BYTES as u64 - TRANSFER_BYTES as u64;

/// Each `SharedReadIntent`'s start, length and source.
fn read_ranges(effects: &[Effect]) -> Vec<(u64, u64, UnitId)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, source } => {
                Some((range.start().raw(), range.length(), *source))
            }
            _ => None,
        })
        .collect()
}

/// A unit whose local store holds `il $10, cmd; wrch $ch21, $10` and
/// whose MFC channels name a `TRANSFER_BYTES` transfer from
/// [`SOURCE_EA`].
fn unit_issuing(cmd: u32) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    s.channels.mfc_lsa = 0x200;
    s.channels.mfc_eah = 0;
    s.channels.mfc_eal = SOURCE_EA as u32;
    s.channels.mfc_size = TRANSFER_BYTES;
    s.channels.mfc_tag_id = 0;
    let il_raw: u32 = 0x081 << 23 | (cmd << 7) | 10;
    s.ls[0..4].copy_from_slice(&il_raw.to_be_bytes());
    let wrch_raw: u32 = 0x10D << 21 | (21u32 << 7) | 10;
    s.ls[4..8].copy_from_slice(&wrch_raw.to_be_bytes());
    unit
}

#[test]
fn getllar_records_the_bytes_it_copied_into_local_store() {
    let mut unit = unit_issuing(MFC_GETLLAR);
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert_eq!(
        read_ranges(&effects),
        [(SOURCE_LINE_EA, u64::from(TRANSFER_BYTES), UnitId::new(UNIT))]
    );
}

#[test]
fn a_transfer_ending_at_the_last_byte_of_memory_records_the_read() {
    let mut unit = unit_issuing(MFC_GETLLAR);
    unit.state_mut().channels.mfc_eal = LAST_LINE_EA as u32;
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert_eq!(
        read_ranges(&effects),
        [(LAST_LINE_EA, u64::from(TRANSFER_BYTES), UnitId::new(UNIT))]
    );
}

#[test]
fn a_getllar_whose_line_does_not_translate_records_no_read() {
    let mut unit = unit_issuing(MFC_GETLLAR);
    unit.state_mut().channels.mfc_eal = LAST_LINE_EA as u32;
    // One byte short of the space the transfer needs, so the copy
    // never happens.
    let mem = GuestMemory::new(MEM_BYTES - 1);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
    // The queued invalid command is why the step records no read. An
    // empty batch alone would also satisfy the assertion above.
    assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::MfcInvalidCommand { .. })),
        "{effects:?}"
    );
}

/// An `MFC_LSA` in the last half-line takes the line that contains it,
/// so nothing wraps and the read is recorded.
#[test]
fn a_getllar_near_the_end_of_local_store_records_its_read() {
    let mut unit = unit_issuing(MFC_GETLLAR);
    let ls_len = unit.state().ls.len() as u32;
    unit.state_mut().channels.mfc_lsa = ls_len - TRANSFER_BYTES / 2;
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert_ne!(result.yield_reason, YieldReason::Fault);
    assert_eq!(read_ranges(&effects).len(), 1, "{effects:?}");
}

#[test]
fn a_step_with_no_transfer_records_no_read() {
    // `il $10, 0`: no channel write, so nothing touches main memory.
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let il_raw: u32 = 0x081 << 23 | 10;
    unit.state_mut().ls[0..4].copy_from_slice(&il_raw.to_be_bytes());
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut effects);

    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
}

/// A get's source is read when the transfer completes, not when the
/// unit issues it, so the issuing step records the queued transfer and
/// no read.
#[test]
fn a_queued_get_records_its_transfer_and_no_read_at_issue() {
    let mut unit = unit_issuing(MFC_GET);
    unit.state_mut().channels.mfc_size = 64;
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let issuing = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(issuing.yield_reason, YieldReason::DmaSubmitted);
    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::DmaEnqueue { request, payload: None }]
                if request.source() == ByteRange::new(GuestAddr::new(SOURCE_EA), 64).unwrap()
        ),
        "{effects:?}"
    );
}

#[test]
fn a_zero_byte_transfer_records_no_read() {
    let mut unit = unit_issuing(MFC_GET);
    unit.state_mut().channels.mfc_size = 0;
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
}

#[test]
fn an_effective_address_at_the_top_of_the_space_records_no_read() {
    let mut unit = unit_issuing(MFC_GET);
    let s = unit.state_mut();
    // ea + 128 carries out of the 64-bit space.
    s.channels.mfc_eah = u32::MAX;
    s.channels.mfc_eal = 0xFFFF_FFC0;
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    let _ = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
}
