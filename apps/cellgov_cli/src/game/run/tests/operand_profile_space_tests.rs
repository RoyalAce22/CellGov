//! The operand profile classifies each address space's units against that
//! space's own memory.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::profile::InsnClass;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::operand_profile_report;

/// `addi r3, r3, 1`.
const ADDI: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `addi r4, r4, 0`: a class of its own, its immediate zero.
const ADDI_ZERO: u32 = (14 << 26) | (4 << 21) | (4 << 16);
/// `sc`, which ends the unit's run.
const SC: u32 = 0x4400_0002;
/// The child space the second unit runs in.
const CHILD: AddressSpaceId = AddressSpaceId::new(1);

/// A memory holding `words` from address 0.
fn memory(words: &[u32]) -> GuestMemory {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap();
    mem.apply_commit(range, &bytes).unwrap();
    mem
}

/// A profiled unit with id `id` that ran `mem`'s code from 0 to its `sc`.
fn ran(id: cellgov_event::UnitId, mem: &GuestMemory) -> PpuExecutionUnit {
    let mut unit = PpuExecutionUnit::new(id);
    let text = &mem.as_bytes()[..8];
    unit.set_instruction_shadow(PredecodedShadow::build(0, text));
    unit.set_profile_mode(true);
    unit.run_until_yield(Budget::new(8), &ExecutionContext::new(mem), &mut Vec::new());
    unit
}

#[test]
fn each_space_s_units_are_classified_against_its_own_words() {
    let boot = memory(&[ADDI, SC]);
    let child = memory(&[ADDI_ZERO, SC]);
    let mut rt = Runtime::new(boot.clone(), Budget::new(1), 1_000);
    rt.create_address_space_with(CHILD, child.clone()).unwrap();
    rt.register_unit_with(|id| ran(id, &boot));
    let child_unit = rt.register_unit_with(|id| ran(id, &child));
    rt.assign_unit_space(child_unit, CHILD).unwrap();

    let report = operand_profile_report(&rt);

    let row = |word| {
        let class = InsnClass::of(word).unwrap();
        report.insns.iter().find(|(c, ..)| *c == class).map(|r| r.1)
    };
    assert_eq!(report.retired, 4, "each unit ran its addi and its sc");
    assert_eq!(row(ADDI), Some(1), "the boot unit's addi");
    assert_eq!(
        row(ADDI_ZERO),
        Some(1),
        "the child unit's addi, read from the child's memory"
    );
}
