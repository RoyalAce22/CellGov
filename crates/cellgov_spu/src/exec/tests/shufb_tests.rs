//! shufb control bytes: the three constant patterns, their boundaries, and the
//! selector bits the byte index ignores.

// [SPU-ISA p:116 s:5 Table 5-1] 10xxxxxx gives 0x00, 110xxxxx gives 0xFF, 111xxxxx gives 0x80, any other byte selects byte (RC & 0x1F) of RA || RB.

use super::*;
use crate::state::SpuState;

/// Runs shufb with every RC byte set to `control`, over RA = 0x00..0x0F and
/// RB = 0x10..0x1F, and returns RT.
fn shufb_all(control: u8) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = std::array::from_fn(|i| i as u8);
    s.regs[2] = std::array::from_fn(|i| 0x10 + i as u8);
    s.regs[3] = [control; 16];
    execute(
        &SpuInstruction::Shufb {
            rt: 4,
            ra: 1,
            rb: 2,
            rc: 3,
        },
        &mut s,
        UnitId::new(0),
    );
    s.regs[4]
}

#[test]
fn shufb_10xxxxxx_gives_zero() {
    for control in [0x80, 0x9F, 0xA0, 0xBF] {
        assert_eq!(shufb_all(control), [0x00; 16], "control {control:#04x}");
    }
}

#[test]
fn shufb_110xxxxx_gives_ff() {
    for control in [0xC0, 0xCF, 0xDF] {
        assert_eq!(shufb_all(control), [0xFF; 16], "control {control:#04x}");
    }
}

#[test]
fn shufb_111xxxxx_gives_80() {
    for control in [0xE0, 0xEF, 0xFF] {
        assert_eq!(shufb_all(control), [0x80; 16], "control {control:#04x}");
    }
}

#[test]
fn shufb_selector_uses_only_the_low_five_bits() {
    // 0x3F and 0x7F have b0 clear, so both address byte 31: RB byte 15.
    assert_eq!(shufb_all(0x1F), [0x1F; 16]);
    assert_eq!(shufb_all(0x3F), [0x1F; 16]);
    assert_eq!(shufb_all(0x7F), [0x1F; 16]);
    assert_eq!(shufb_all(0x60), [0x00; 16]);
    assert_eq!(shufb_all(0x70), [0x10; 16]);
}

#[test]
fn shufb_mixes_constants_and_selectors_per_byte() {
    let mut s = SpuState::new();
    s.regs[1] = std::array::from_fn(|i| 0xA0 + i as u8);
    s.regs[2] = std::array::from_fn(|i| 0xB0 + i as u8);
    s.regs[3] = [
        0x80, 0xC0, 0xE0, 0x00, 0x1F, 0x10, 0xBF, 0xDF, 0xFF, 0x7F, 0x3F, 0x0F, 0x9A, 0xC5, 0xE9,
        0x05,
    ];
    execute(
        &SpuInstruction::Shufb {
            rt: 4,
            ra: 1,
            rb: 2,
            rc: 3,
        },
        &mut s,
        UnitId::new(0),
    );
    assert_eq!(
        s.regs[4],
        [
            0x00, 0xFF, 0x80, 0xA0, 0xBF, 0xB0, 0x00, 0xFF, 0x80, 0xBF, 0xBF, 0xAF, 0x00, 0xFF,
            0x80, 0xA5,
        ]
    );
}
