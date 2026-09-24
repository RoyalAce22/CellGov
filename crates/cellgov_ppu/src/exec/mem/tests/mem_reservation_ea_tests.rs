//! Store-conditional at an effective address past the Cell EA space.

use super::*;

/// Runs a store-conditional at `(1 << 50) + 0x80` while the unit holds a
/// reservation on the line at 0x1000.
fn store_conditional(insn: PpuInstruction) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(4, 1 << 50);
    s.set_gpr(5, 0x80);
    s.set_gpr(6, 0x1122_3344_5566_7788);
    s.set_reservation(Some(ReservedLine::containing(0x1000)));
    let mut effects = Vec::new();
    let v = exec_with_mem(&insn, &mut s, 0, &[0u8; 0x2000], &mut effects);
    (v, s, effects)
}

// [PPC-Book2 p:25 s:3.3] A store-conditional whose EA is outside the reserved line fails: CR0 = 0b00 || 0 || XER[SO], and the reservation clears.
#[test]
fn a_store_conditional_past_the_ea_space_fails_without_storing() {
    for insn in [
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
    ] {
        let (v, s, effects) = store_conditional(insn);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
        assert_eq!(s.cr_field(0), 0, "{insn:?}");
        assert_eq!(s.reservation(), None, "{insn:?}");
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}
