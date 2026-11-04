//! The mnemonic of each word the PPE refuses
//! ([`crate::instruction::PpuInstruction::PpeIllegal`] and
//! [`crate::instruction::PpuInstruction::PpePrivileged`]), and the SPR
//! number that an SPR move names.

use cellgov_ps3_abi::hw::ppu_spr::{spr_read_class, spr_write_class, SprClass};

/// The mnemonic of a [`super::PpuInstruction::PpeIllegal`] word, or `None` for any other word.
///
/// `fre` and `frsqrtes` key on the 5-bit A-form XO, so FRC and Rc do
/// not change the answer.
pub const fn ppe_illegal_mnemonic(raw: u32) -> Option<&'static str> {
    let xo_10 = (raw >> 1) & 0x3FF;
    let xo_5 = xo_10 & 0x1F;
    match raw >> 26 {
        31 => match xo_10 {
            310 => Some("eciwx"),
            438 => Some("ecowx"),
            370 => Some("tlbia"),
            210 => Some("mtsr"),
            242 => Some("mtsrin"),
            595 => Some("mfsr"),
            659 => Some("mfsrin"),
            339 if matches!(spr_read_class(spr_number(raw)), SprClass::Reserved) => Some("mfspr"),
            467 if matches!(spr_write_class(spr_number(raw)), SprClass::Reserved) => Some("mtspr"),
            _ => None,
        },
        63 if xo_5 == 24 => Some("fre"),
        59 if xo_5 == 26 => Some("frsqrtes"),
        // [PPC-Book3 p:92 s:8.4] An lq with RT = RA invokes the illegal instruction error handler.
        56 if (raw >> 21) & 0x1F == (raw >> 16) & 0x1F => Some("lq"),
        _ => None,
    }
}

/// The mnemonic of a [`super::PpuInstruction::PpePrivileged`] word, or `None` for any other word.
pub const fn ppe_privileged_mnemonic(raw: u32) -> Option<&'static str> {
    let xo_10 = (raw >> 1) & 0x3FF;
    match raw >> 26 {
        19 => match xo_10 {
            18 => Some("rfid"),
            274 => Some("hrfid"),
            _ => None,
        },
        31 => match xo_10 {
            83 => Some("mfmsr"),
            146 => Some("mtmsr"),
            178 => Some("mtmsrd"),
            498 => Some("slbia"),
            434 => Some("slbie"),
            402 => Some("slbmte"),
            851 => Some("slbmfev"),
            915 => Some("slbmfee"),
            306 => Some("tlbie"),
            274 => Some("tlbiel"),
            566 => Some("tlbsync"),
            339 if matches!(spr_read_class(spr_number(raw)), SprClass::Privileged) => Some("mfspr"),
            467 if matches!(spr_write_class(spr_number(raw)), SprClass::Privileged) => {
                Some("mtspr")
            }
            _ => None,
        },
        // [PPC-Book3 p:92 s:8.4] lq and stq are privileged.
        56 if (raw >> 21) & 0x1F != (raw >> 16) & 0x1F => Some("lq"),
        62 if raw & 0x3 == 2 => Some("stq"),
        _ => None,
    }
}

/// The SPR number of an XFX-form `mfspr` or `mtspr` word: the two
/// 5-bit halves of the field, swapped back into reading order.
/// [PPC-Book1 p:9 s:1.7.8 XFX-Form] spr occupies bits 11:20 with its halves reversed.
pub const fn spr_number(raw: u32) -> u16 {
    ((((raw >> 11) & 0x1F) << 5) | ((raw >> 16) & 0x1F)) as u16
}
