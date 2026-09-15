//! A fault's detail cannot name a different class.
//!
//! A code is a class in the high half and a detail in the low. Each
//! detail these paths carry is a value the guest picked or influenced --
//! a program counter, a channel number, an MFC command word, a tag id --
//! so a detail wide enough to set a class bit would make the code decode
//! as some other fault.
//!
//! The layout itself -- local store wider than the detail half, no class
//! reaching into that half, no two classes sharing a code -- is a
//! compile-time check beside `EVERY_FAULT_CLASS`, so it holds in every
//! profile. What is left for a test is the mapping: that each raised
//! class survives the widest detail its field allows, and that the list
//! the check reads names the classes the code actually raises.

use crate::exec::SpuFault;
use crate::{
    SpuExecutionUnit, EVERY_FAULT_CLASS, FAULT_DETAIL_MASK, FAULT_LS_OUT_OF_RANGE,
    FAULT_MFC_TAG_ID_OUT_OF_RANGE, FAULT_UNSUPPORTED_CHANNEL, FAULT_UNSUPPORTED_CHANNEL_COUNT,
    FAULT_UNSUPPORTED_MFC_CMD,
};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_LS_SIZE;
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// `nop`, RR opcode 0x201 in the high 11 bits.
const NOP: u32 = 0x201 << 21;

fn code_for(fault: SpuFault) -> u32 {
    match crate::guest_fault_for(fault) {
        FaultKind::Guest(code) => code,
        other => panic!("expected a guest fault, got {other:?}"),
    }
}

/// Each [`SpuFault`] variant at the widest detail its own field allows,
/// and the class that detail must leave alone.
///
/// Two reach the class field from a guest: the MFC command word and the
/// staged tag id are whole 32-bit channel writes. `UnsupportedChannel`
/// and `UnsupportedChannelCount` read a channel out of a 7-bit
/// instruction field, so `u8::MAX` is past anything a program produces
/// and stands here for the field's own bound. `LsOutOfRange` carries the
/// raw address operand, which a guest picks freely, but `exec::ls_addr`
/// masks that operand to 0x3FFF0 before its bounds test, so a whole
/// local store never raises the variant; the fetch path raises that
/// class instead, which the last test walks to.
fn every_variant_at_its_widest_detail() -> Vec<(SpuFault, u32)> {
    vec![
        (SpuFault::LsOutOfRange(u32::MAX), FAULT_LS_OUT_OF_RANGE),
        (
            SpuFault::UnsupportedChannel {
                channel: u8::MAX,
                is_write: true,
            },
            FAULT_UNSUPPORTED_CHANNEL,
        ),
        (
            SpuFault::UnsupportedMfcCommand(u32::MAX),
            FAULT_UNSUPPORTED_MFC_CMD,
        ),
        (
            SpuFault::UnsupportedChannelCount(u8::MAX),
            FAULT_UNSUPPORTED_CHANNEL_COUNT,
        ),
        (
            SpuFault::TagIdOutOfRange(u32::MAX),
            FAULT_MFC_TAG_ID_OUT_OF_RANGE,
        ),
    ]
}

#[test]
fn no_class_is_impersonated_by_its_detail() {
    for (fault, want) in every_variant_at_its_widest_detail() {
        let code = code_for(fault.clone());
        assert_eq!(
            code & !FAULT_DETAIL_MASK,
            want,
            "{fault:?} decoded as a different class: 0x{code:08x}",
        );
    }
}

/// The compile-time layout check reads `EVERY_FAULT_CLASS`, so nothing
/// covers a class the code raises and the list omits.
#[test]
fn every_raised_class_is_in_the_checked_list() {
    for (fault, _) in every_variant_at_its_widest_detail() {
        let class = code_for(fault.clone()) & !FAULT_DETAIL_MASK;
        assert!(
            EVERY_FAULT_CLASS.contains(&class),
            "{fault:?} raises 0x{class:08x}, which the layout check does not cover",
        );
    }
}

/// The reachable case, end to end: a guest that steps off the last
/// instruction reports the out-of-range class, not another one.
///
/// The fetch path's detail is the raw `pc`, and branches mask `pc` to
/// 0x3FFFC, so the value a step off the end produces is `SPU_LS_SIZE`.
/// Unmasked it ORs into the class field and the code reads as
/// `FAULT_UNSUPPORTED_CHANNEL_COUNT`.
#[test]
fn a_guest_that_steps_off_local_store_keeps_its_own_class() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let last = SPU_LS_SIZE - 4;
    unit.state_mut().ls[last..].copy_from_slice(&NOP.to_be_bytes());
    unit.state_mut().pc = last as u32;

    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);

    assert_eq!(
        unit.state().pc,
        SPU_LS_SIZE as u32,
        "the nop retired and carried pc past the last instruction",
    );
    assert_eq!(
        result.yield_reason,
        YieldReason::Fault,
        "there is no instruction at that address",
    );
    let Some(FaultKind::Guest(code)) = result.fault else {
        panic!("expected a guest fault, got {:?}", result.fault);
    };
    assert_ne!(
        SPU_LS_SIZE as u32 & !FAULT_DETAIL_MASK,
        0,
        "the premise: the detail this path carries reaches the class field",
    );
    assert_eq!(
        code & !FAULT_DETAIL_MASK,
        FAULT_LS_OUT_OF_RANGE,
        "the class a reader decodes is the one raised: 0x{code:08x}",
    );
    assert_eq!(
        code & FAULT_DETAIL_MASK,
        0,
        "the detail is the address modulo 64 KB, which is zero for this one",
    );
    assert_eq!(
        result.local_diagnostics.pc,
        Some(SPU_LS_SIZE as u64),
        "the address the detail cannot hold travels beside the code",
    );
}
