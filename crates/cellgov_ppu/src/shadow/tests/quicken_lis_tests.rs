//! `addis rT, 0, imm` quickens to `Lis`; an `addis` with a base register
//! stays an add.

use super::quicken_insn;
use crate::exec::execute;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The value `insn` leaves in `rt` when every GPR starts as `seed`.
fn result(insn: I, rt: usize, seed: u64) -> u64 {
    let mut state = PpuState::new();
    for r in 0..32 {
        state.set_gpr(r, seed);
    }
    execute(
        &insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    state.gpr[rt]
}

#[test]
fn addis_with_ra_0_is_lis() {
    let raw = I::Addis {
        rt: 3,
        ra: 0,
        imm: -2,
    };
    assert_eq!(quicken_insn(raw), Some(I::Lis { rt: 3, imm: -2 }));
}

#[test]
fn lis_sign_extends_the_shifted_immediate() {
    let lis = I::Lis {
        rt: 3,
        imm: i16::MIN,
    };
    // [PPC-Book1 p:51 s:3.3.8] addis with RA = 0: RT <- EXTS(SI || 0x0000).
    assert_eq!(result(lis, 3, 7), 0xFFFF_FFFF_8000_0000);
}

#[test]
fn addis_with_a_base_register_stays_an_add() {
    let raw = I::Addis {
        rt: 3,
        ra: 4,
        imm: 1,
    };
    assert_eq!(quicken_insn(raw), None);
}
