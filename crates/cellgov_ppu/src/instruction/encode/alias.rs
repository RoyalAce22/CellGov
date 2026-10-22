//! The second spellings a raw word can carry for a decoded instruction.

use cellgov_ps3_abi::hw::ppc_isa::PPC_STORAGE_HINT_XOS;

#[cfg(doc)]
use super::reserved_bits;

/// The second spelling of a decoded instruction that a raw word can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alias {
    /// A cache hint the deterministic model decodes as `nop`.
    NopHint {
        /// Primary opcode of the hint.
        primary: u8,
        /// Extended opcode of the hint.
        xo: u16,
    },
    /// `mfspr` naming the time base, which `mftb` also reads.
    TimeBaseThroughMfspr {
        /// Time-base register number, 268 or 269.
        tbr: u16,
    },
}

/// Names the second spelling `raw` uses when it is not the canonical word.
///
/// The deterministic model decodes the cache hints as `nop`. Returns
/// `None` for a word whose only difference from its canonical encoding
/// is in [`reserved_bits`].
pub fn alias(raw: u32) -> Option<Alias> {
    let primary = (raw >> 26) as u8;
    let xo = ((raw >> 1) & 0x3FF) as u16;
    match primary {
        31 if PPC_STORAGE_HINT_XOS.contains(&u32::from(xo)) => Some(Alias::NopHint { primary, xo }),
        31 if xo == 339 => {
            let spr = (((raw >> 11) & 0x1F) << 5 | ((raw >> 16) & 0x1F)) as u16;
            (spr == 268 || spr == 269).then_some(Alias::TimeBaseThroughMfspr { tbr: spr })
        }
        _ => None,
    }
}
