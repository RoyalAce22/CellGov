//! The hypervisor answers the 4.93 kernel's boot call with the spe id and
//! the shadow page that the reference console's kernel holds, read from
//! the committed capture `tests/micro/lv2_logical_spe_object`.

use super::*;
use crate::{AddressSpace, AddressSpaceChange, MemoryRegion};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::spe::LOGICAL_SPE_OBJECT_CAPTURE;
use cellgov_time::GuestTicks;

/// The vas id of the 4.93 kernel's boot address space, which the console
/// capture `lv2_vas_boot_record` reads.
const VAS_ID: u64 = 2;

/// The value of the field `field` in the capture. Both reads of the word
/// must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LOGICAL_SPE_OBJECT_CAPTURE)
        .join("peek/cech20-cex-493/spe_object.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!((row[3], row[4]), ("2", "yes"), "{name}: {field} reads");
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// The memory state that the 4.93 kernel's boot leaves before the call:
/// the two boot allocations, and the boot address space with its page
/// table mapped. Each takes the next region counter value from the boot
/// value.
fn boot_memory() -> MemoryGroupState {
    let boot = game_os_region_counter_at_boot("4.93").expect("the 4.93 boot counter");
    let mut memory = MemoryGroupState::default();
    for (counter, size, page_size_exp) in [(boot, 0x10_0000, 20), (boot + 1, 0x1_0000, 16)] {
        memory.apply(MemoryChange::Region(MemoryRegion {
            lpar_address: region_address(size, counter).expect("an address"),
            size,
            page_size_exp,
            unit_id: 1,
            counter,
        }));
    }
    memory.apply(MemoryChange::AddressSpace(AddressSpaceChange::Constructed(
        AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        },
    )));
    let table = region_address(0x4_0000, boot + 2).expect("the page table mapping");
    memory.apply(MemoryChange::AddressSpace(AddressSpaceChange::Mapped {
        vas_id: VAS_ID,
        lpar_address: table,
        counter: boot + 2,
    }));
    memory
}

/// The kernel's boot call [OFW-4.93 lv2_kernel a:0x800000000004f188],
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

#[test]
fn the_kernels_boot_call_gets_the_captured_spe_id_and_shadow_page() {
    let d = construct(
        &boot_call(),
        Some("4.93"),
        &boot_memory(),
        &LogicalSpeState::default(),
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.r3, 0);
    let outputs = answer.outputs.expect("r4 to r10");
    assert_eq!(outputs[4], captured("shadow"), "r8");
    assert_eq!(outputs[5], captured("spe_id"), "r9");
}
