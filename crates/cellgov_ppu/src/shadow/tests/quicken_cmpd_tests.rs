//! `cmpdi crF, rA, 0` quickens to `CmpdZero`, which compares all 64 bits
//! of rA; a nonzero immediate stays a `Cmpdi`.

use super::quicken_insn;
use crate::exec::execute;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// CR field 3 after `insn` runs with r4 = `value` and XER[SO] = `so`.
fn field3(insn: I, value: u64, so: bool) -> u8 {
    let mut state = PpuState::new();
    state.set_gpr(4, value);
    // XER[SO] is PPC bit 32, Rust bit 31.
    state.set_xer(u64::from(so) << 31);
    execute(
        &insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    state.cr_field(3)
}

const CMPD_ZERO: I = I::CmpdZero { bf: 3, ra: 4 };

#[test]
fn cmpdi_against_zero_is_cmpd_zero() {
    let raw = I::Cmpdi {
        bf: 3,
        ra: 4,
        imm: 0,
    };
    assert_eq!(quicken_insn(raw), Some(CMPD_ZERO));
}

#[test]
fn cmpdi_against_a_nonzero_immediate_stays_a_compare() {
    let raw = I::Cmpdi {
        bf: 3,
        ra: 4,
        imm: 1,
    };
    assert_eq!(quicken_insn(raw), None);
}

/// [PPC-Book1 p:60 s:3.3.9] cmpi with L = 1 compares (RA) as a signed
/// 64-bit value; the CR field is LT GT EQ SO.
#[test]
fn cmpd_zero_reads_the_high_word() {
    assert_eq!(field3(CMPD_ZERO, 0xFFFF_FFFF_0000_0000, false), 0b1000);
    assert_eq!(field3(CMPD_ZERO, 0x0000_0000_8000_0000, false), 0b0100);
    assert_eq!(field3(CMPD_ZERO, 0, false), 0b0010);
}

#[test]
fn cmpd_zero_copies_summary_overflow() {
    assert_eq!(field3(CMPD_ZERO, 0, true), 0b0011);
}
