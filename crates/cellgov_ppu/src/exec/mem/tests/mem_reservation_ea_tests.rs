//! Store-conditional at an effective address past the Cell EA space.

use super::*;

/// The store-conditional's effective address: no view holds it.
const EA: u64 = (1 << 50) + 0x80;

/// Runs a store-conditional at `EA` while the unit holds a reservation
/// on the line at 0x1000 and CR0 reads 0b0100.
fn store_conditional(insn: PpuInstruction) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    s.set_gpr(4, 1 << 50);
    s.set_gpr(5, 0x80);
    s.set_gpr(6, 0x1122_3344_5566_7788);
    s.set_cr_field(0, 0b0100);
    s.set_reservation(Some(ReservedLine::containing(0x1000)));
    let mut effects = Vec::new();
    let v = exec_with_mem(&insn, &mut s, 0, &[0u8; 0x2000], &mut effects);
    (v, s, effects)
}

/// [CBE-Handbook p:249 s:9.5.3] The PPE raises the data storage
/// interrupt on a store-conditional whether or not the store would
/// update storage.
#[test]
fn a_store_conditional_past_the_ea_space_faults_without_a_reservation_on_its_line() {
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
        assert_eq!(
            v,
            ExecuteVerdict::StoreFault(crate::exec::memory_helpers::unmapped(EA)),
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
