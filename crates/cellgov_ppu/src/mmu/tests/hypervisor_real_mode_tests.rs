//! Real addressing mode with MSR\[HV\] = 1, which CellGov refuses by
//! name.
//!
//! [PPC-Book3 p:29 s:4.2.6] With MSR\[HV\] = 1, the HRMOR offsets an access with EA bit 0 = 0, and an access with EA bit 0 = 1 uses EA bits 2:63 as the real address.

use std::cell::Cell;

use cellgov_ps3_abi::hw::ppu_mmu::{PTEG_BYTES, PTE_V};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_HV, MSR_IR, MSR_SF, SLB_VALID};

use super::translate::HYPERVISOR_REAL_MODE;
use super::*;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::supervisor::{SlbEntry, SupervisorState};

/// A hypervisor-state thread with MSR `msr` and a real mode limit of
/// 256 MB, so an EA with bit 0 = 0 below the limit is one that offset
/// real mode would translate.
fn thread(msr: u64) -> SupervisorState {
    let mut s = SupervisorState::new(MSR_SF | MSR_HV | msr);
    s.partition = Partition::new(0, 1 << 28, 0, Partition::INITIAL.lb()).expect("partition");
    s
}

#[test]
fn every_real_mode_access_with_hv_set_stops_unmodelled_for_either_ea_bit_0() {
    // The SLB maps both segments to VSID 0x123, and every group holds an
    // entry for page 1 of that VSID with PP 10: virtual mode would
    // translate each access below, read a group and give its entry.
    let mut s = thread(0);
    for (i, ea) in [0u64, 0x8000_0000_0000_0000].into_iter().enumerate() {
        s.slb[i] = SlbEntry {
            esid_v: ea | SLB_VALID,
            vsid_flags: 0x123 << 12,
        };
    }
    let mut group = [0u8; PTEG_BYTES];
    group[..8].copy_from_slice(&((0x123u64 << 12) | PTE_V).to_be_bytes());
    group[8..16].copy_from_slice(&(0x0234_5000u64 | 0b10).to_be_bytes());
    let reads = Cell::new(0u32);
    let pteg = |_: u64| -> Option<[u8; PTEG_BYTES]> {
        reads.set(reads.get() + 1);
        Some(group)
    };
    let cases = [
        (0x0000_0000_0000_1234, Access::Load),
        (0x0000_0000_0000_1234, Access::Store),
        (0x0000_0000_0000_1234, Access::Fetch),
        (0x8000_0000_0000_1234, Access::Load),
        (0x8000_0000_0000_1234, Access::Store),
        (0x8000_0000_0000_1234, Access::Fetch),
    ];
    for (ea, access) in cases {
        assert_eq!(
            translate_entry(ea, access, &s, pteg),
            (Err(PpuFault::Unmodelled(HYPERVISOR_REAL_MODE)), None),
            "{access:?} at 0x{ea:016x}"
        );
    }
    assert_eq!(
        reads.get(),
        0,
        "a refused real mode access searched a table"
    );
}

#[test]
fn hv_set_refuses_only_the_side_with_relocation_off() {
    // IR = 1 and DR = 0: a fetch searches the SLB, which holds no entry,
    // and a load is a real mode access.
    let s = thread(MSR_IR);
    let ea = 0x0000_0000_0000_1234;
    assert_eq!(
        translate_entry(ea, Access::Fetch, &s, |_| None).0,
        Err(PpuInterrupt::InstructionSegment.into())
    );
    assert_eq!(
        translate_entry(ea, Access::Load, &s, |_| None).0,
        Err(PpuFault::Unmodelled(HYPERVISOR_REAL_MODE))
    );
    let s = thread(MSR_DR);
    assert_eq!(
        translate_entry(ea, Access::Load, &s, |_| None).0,
        Err(PpuInterrupt::DataSegment { ea, store: false }.into())
    );
    assert_eq!(
        translate_entry(ea, Access::Fetch, &s, |_| None).0,
        Err(PpuFault::Unmodelled(HYPERVISOR_REAL_MODE))
    );
}
