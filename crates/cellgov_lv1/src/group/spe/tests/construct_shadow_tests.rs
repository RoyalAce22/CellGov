//! The page of CellGov's shadow page storage that a construct gives the
//! new SPE.

use super::*;
use crate::{
    AddressSpace, AddressSpaceChange, StateChange, SHADOW_STORAGE_BASE, SHADOW_STORAGE_PAGES,
};
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The vas id of the 4.93 kernel's boot address space, which the console
/// capture `lv2_vas_boot_record` reads.
const VAS_ID: u64 = 2;

/// The 4.93 kernel's boot call [OFW-4.93 lv2_kernel a:0x800000000004f188],
/// with r3 to r9 as it sets them
/// [OFW-4.93 lv2_kernel a:0x800000000004f158-0x800000000004f184].
fn boot_call() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_LOGICAL_SPE,
        inputs: [12, 12, 12, 12, 12, VAS_ID, 1, 0],
        pc: 0x8000_0000_0004_f188,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

/// A memory state that holds the boot address space.
fn memory() -> MemoryGroupState {
    let mut memory = MemoryGroupState::default();
    memory.apply(MemoryChange::AddressSpace(AddressSpaceChange::Constructed(
        AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        },
    )));
    memory
}

/// A state that holds `count` SPEs whose ids are not the boot call's 11.
fn held(count: u64) -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    for index in 0..count {
        spes.construct(LogicalSpe {
            id: 100 + index,
            spe_type: 1,
            vas_id: VAS_ID,
            area: 0,
            shadow: 0,
            shadow_real: SHADOW_STORAGE_BASE + (index << 12),
        });
    }
    spes
}

fn constructed_shadow_real(spes: &LogicalSpeState) -> Option<u64> {
    let HypercallDisposition::Answered(answer) =
        construct(&boot_call(), Some("4.93"), &memory(), spes)
    else {
        return None;
    };
    answer.changes.iter().find_map(|change| match change {
        StateChange::Spe(change) => match **change {
            SpeChange::Constructed(spe) => Some(spe.shadow_real),
            _ => None,
        },
        _ => None,
    })
}

#[test]
fn each_new_spe_gets_the_storage_page_after_the_held_spes() {
    let cases = [0, 1, SHADOW_STORAGE_PAGES - 1];
    assert_eq!(cases.len(), 3);
    for count in cases {
        assert_eq!(
            constructed_shadow_real(&held(count)),
            Some(SHADOW_STORAGE_BASE + (count << 12)),
            "{count} held"
        );
    }
}

#[test]
fn a_construct_past_the_shadow_page_storage_is_refused() {
    let r = boot_call();
    assert_eq!(
        construct(&r, Some("4.93"), &memory(), &held(SHADOW_STORAGE_PAGES)),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONSTRUCT_LOGICAL_SPE,
            unit: r.unit,
            pc: r.pc,
            what: "a logical SPE construct past CellGov's shadow page storage",
        })
    );
}
