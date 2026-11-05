//! Each fuzz descriptor whose outcome can be a fault names the fault
//! classes its producer can raise.

use super::registry::generation_descriptors;
use super::types::PpuOutcomeClass;
use crate::instruction::PpuInstruction;
use crate::PpuFaultClass as C;

/// The fault classes of the descriptor of the instruction `raw` decodes to.
fn faults(raw: u32) -> &'static [C] {
    let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
    insn.fuzz_descriptor(raw).faults
}

#[test]
fn a_descriptor_names_fault_classes_exactly_when_an_outcome_is_a_fault() {
    let mut faulting = 0;
    for generation in generation_descriptors() {
        let raw = generation.canonical_word;
        let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
        let descriptor = insn.fuzz_descriptor(raw);
        let memory = descriptor.outcomes.contains(&PpuOutcomeClass::MemoryFault);
        let fault = descriptor.outcomes.contains(&PpuOutcomeClass::Fault);
        assert_eq!(
            !descriptor.faults.is_empty(),
            memory || fault,
            "{:?} 0x{raw:08x}",
            descriptor.kind
        );
        assert_eq!(
            descriptor.faults.contains(&C::DataStorage),
            memory,
            "{:?} 0x{raw:08x}",
            descriptor.kind
        );
        faulting += usize::from(memory || fault);
    }
    assert!(faulting > 100, "only {faulting} descriptors can fault");
}

#[test]
fn each_producer_names_the_class_it_raises() {
    let cases: [(&str, u32, &[C]); 10] = [
        ("addi", 14 << 26, &[]),
        ("lwz", 32 << 26, &[C::DataStorage]),
        (
            "lwzu r3,0(r0)",
            (33 << 26) | (3 << 21),
            &[C::ProgramIllegal],
        ),
        (
            "lwarx",
            (31 << 26) | (20 << 1),
            &[C::Alignment, C::DataStorage],
        ),
        ("tw", (31 << 26) | (31 << 21) | (4 << 1), &[C::ProgramTrap]),
        ("mfmsr", 0x7C60_00A6, &[C::ProgramPrivileged]),
        ("popcntb", (31 << 26) | (122 << 1), &[C::ProgramIllegal]),
        ("sc", 0x4400_0002, &[]),
        ("sc 1", 0x4400_0022, &[C::SystemCall]),
        (
            "dcbz",
            (31 << 26) | (1014 << 1),
            &[C::Alignment, C::DataStorage],
        ),
    ];
    for (name, raw, want) in cases {
        assert_eq!(faults(raw), want, "{name}");
    }
}

#[test]
fn a_vx_op_the_interpreter_executes_names_no_fault_class() {
    let vaddubm = (4 << 26) | (3 << 21) | (1 << 16) | (2 << 11);
    assert!(matches!(
        crate::decode::decode(vaddubm),
        Ok(PpuInstruction::Vx { .. })
    ));
    assert_eq!(faults(vaddubm), &[] as &[C]);
}
