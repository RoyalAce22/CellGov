//! Store-conditional to an effective address no region holds.

use super::*;

/// The first byte past the `0x2000`-byte region the tests map at 0.
const UNMAPPED: u64 = 0x2000;

/// Runs `insn` with r4 + r5 = `ea` and the given reservation while CR0
/// reads 0b0100.
fn run(
    insn: PpuInstruction,
    ea: u64,
    reservation: Option<ReservedLine>,
) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(4, ea);
    s.set_gpr(5, 0);
    s.set_gpr(6, 0x1122_3344_5566_7788);
    s.set_cr_field(0, 0b0100);
    s.set_reservation(reservation);
    let mut effects = Vec::new();
    let v = exec_with_mem(&insn, &mut s, 0, &[0u8; 0x2000], &mut effects);
    (v, s, effects)
}

fn both() -> [PpuInstruction; 2] {
    [
        PpuInstruction::Stwcx {
            rs: 6,
            ra: 4,
            rb: 5,
        },
        PpuInstruction::Stdcx {
            rs: 6,
            ra: 4,
            rb: 5,
        },
    ]
}

/// [CBE-Handbook p:249 s:9.5.3] The PPE raises the data storage
/// interrupt on a store-conditional whether or not the store would
/// update storage.
#[test]
fn a_store_conditional_with_no_reservation_faults_on_an_unmapped_ea() {
    for insn in both() {
        let (v, s, effects) = run(insn, UNMAPPED, None);
        assert_eq!(
            v,
            ExecuteVerdict::StoreFault(crate::exec::memory_helpers::unmapped(UNMAPPED)),
            "{insn:?}"
        );
        assert_eq!(s.cr_field(0), 0b0100, "{insn:?}");
        assert_eq!(s.reservation(), None, "{insn:?}");
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

/// [CBE-Handbook p:253 s:9.5.8] A misaligned store-conditional takes the
/// alignment interrupt. [CBE-Handbook p:290 s:9.10] That interrupt ranks
/// above the data storage interrupt for a page fault.
#[test]
fn a_misaligned_store_conditional_to_an_unmapped_ea_raises_the_alignment_fault() {
    for insn in both() {
        let ea = UNMAPPED + 2;
        let (v, s, effects) = run(insn, ea, Some(ReservedLine::containing(0x1000)));
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::alignment(ea, true)),
            "{insn:?}"
        );
        assert_eq!(s.cr_field(0), 0b0100, "{insn:?}");
        assert_eq!(
            s.reservation(),
            Some(ReservedLine::containing(0x1000)),
            "{insn:?}"
        );
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}
