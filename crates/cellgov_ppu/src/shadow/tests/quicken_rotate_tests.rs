//! Every non-wrapping `rlwinm` quickens, and every rotate quickening leaves
//! the result the raw rotate gives; the named forms keep their shapes.

use super::quicken_insn;
use crate::exec::execute;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Register values with a set sign bit, a set low bit, and a high word
/// that differs from the low word.
const VALUES: [u64; 4] = [
    0x0123_4567_89AB_CDEF,
    0xFFFF_FFFF_0000_0001,
    0x0000_0000_8000_0000,
    u64::MAX,
];

/// RA after `insn` runs with RS (r4) holding `value`.
fn result(insn: I, value: u64) -> u64 {
    let mut state = PpuState::new();
    state.set_gpr(4, value);
    execute(
        &insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    state.gpr[3]
}

/// Asserts the quickened form of `raw`, if any, gives the raw result on
/// every value, and returns whether `raw` quickened.
fn quickens_to_the_same_result(raw: I) -> bool {
    let Some(quick) = quicken_insn(raw) else {
        return false;
    };
    for value in VALUES {
        assert_eq!(
            result(quick, value),
            result(raw, value),
            "{raw:?} on {value:#x}"
        );
    }
    true
}

#[test]
fn every_rlwinm_quickens_to_the_raw_result_unless_its_mask_wraps() {
    let mut quickened = 0u32;
    for sh in 0..32u8 {
        for mb in 0..32u8 {
            for me in 0..32u8 {
                let raw = I::Rlwinm {
                    ra: 3,
                    rs: 4,
                    sh,
                    mb,
                    me,
                    rc: false,
                };
                let did = quickens_to_the_same_result(raw);
                assert_eq!(did, mb <= me, "{raw:?}");
                quickened += u32::from(did);
            }
        }
    }
    // 32 shifts times the 528 (MB, ME) pairs with MB <= ME.
    assert_eq!(quickened, 32 * 528);
}

#[test]
fn every_rldicl_and_rldicr_quickening_gives_the_raw_result() {
    let mut quickened = 0u32;
    for sh in 0..64u8 {
        for m in 0..64u8 {
            let rldicl = I::Rldicl {
                ra: 3,
                rs: 4,
                sh,
                mb: m,
                rc: false,
            };
            let rldicr = I::Rldicr {
                ra: 3,
                rs: 4,
                sh,
                me: m,
                rc: false,
            };
            quickened += u32::from(quickens_to_the_same_result(rldicl));
            quickened += u32::from(quickens_to_the_same_result(rldicr));
        }
    }
    // clrldi: 64 (SH = 0); srdi: 63 (SH != 0, MB = 64 - SH); sldi: 63.
    assert_eq!(quickened, 64 + 63 + 63);
}

/// [PPC-Book1 p:160 s:B.7.2 Table 10] rotlwi ra,rs,n == rlwinm ra,rs,n,0,31.
#[test]
fn rotlwi_is_rlwinm_mask_with_the_full_word_mask() {
    let raw = I::Rlwinm {
        ra: 3,
        rs: 4,
        sh: 8,
        mb: 0,
        me: 31,
        rc: false,
    };
    let quick = I::RlwinmMask {
        ra: 3,
        rs: 4,
        sh: 8,
        mb: 0,
        me: 31,
        mask: u32::MAX,
    };
    assert_eq!(quicken_insn(raw), Some(quick));
    assert_eq!(result(quick, 0xFFFF_FFFF_1234_5678), 0x3456_7812);
}

/// [PPC-Book1 p:160 s:B.7.2 Table 10] srwi 4 == rlwinm 28,4,31;
/// clrlwi 0 == rlwinm 0,0,31; slwi 4 == rlwinm 4,0,27.
#[test]
fn the_named_rotate_forms_still_match_first() {
    let rlwinm = |sh, mb, me| I::Rlwinm {
        ra: 3,
        rs: 4,
        sh,
        mb,
        me,
        rc: false,
    };
    assert_eq!(
        quicken_insn(rlwinm(28, 4, 31)),
        Some(I::Srwi { ra: 3, rs: 4, n: 4 })
    );
    assert_eq!(
        quicken_insn(rlwinm(0, 0, 31)),
        Some(I::Clrlwi { ra: 3, rs: 4, n: 0 })
    );
    assert_eq!(
        quicken_insn(rlwinm(4, 0, 27)),
        Some(I::Slwi { ra: 3, rs: 4, n: 4 })
    );
}

#[test]
fn a_wrapping_or_recording_rlwinm_stays_generic() {
    let wrapping = I::Rlwinm {
        ra: 3,
        rs: 4,
        sh: 8,
        mb: 28,
        me: 3,
        rc: false,
    };
    let recording = I::Rlwinm {
        ra: 3,
        rs: 4,
        sh: 8,
        mb: 0,
        me: 31,
        rc: true,
    };
    assert_eq!(quicken_insn(wrapping), None);
    assert_eq!(quicken_insn(recording), None);
}
