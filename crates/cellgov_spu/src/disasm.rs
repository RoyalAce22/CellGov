//! One SPU instruction word as text, and how the decoder treats it.
//!
//! The mnemonic and the operand fields come from the opcode map's row
//! for the word: each form places its fields at fixed bits, so the text
//! names the fields the word carries, registers as `$n` and immediates
//! as their raw field value in hex.
//!
//! [SPU-ISA p:28 s:2.3] the RR, RRR and RI7 formats.
//! [SPU-ISA p:29 s:2.3] the RI10, RI16 and RI18 formats.

use std::fmt;

use cellgov_ps3_abi::hw::spu_isa::{self, SpuForm, SpuOpcodeRow};

use crate::instruction::SpuDecodeError;

/// How the decoder treats one instruction word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpuWordClass {
    /// An instruction CellGov decodes and runs.
    Implemented,
    /// An instruction the ISA defines and CellGov does not implement.
    NotImplemented,
    /// An optional ISA instruction the CBE does not provide.
    AbsentOnCbe,
    /// A word that is no SPU instruction.
    Unassigned,
}

impl SpuWordClass {
    /// The class's name in census output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::NotImplemented => "not-implemented",
            Self::AbsentOnCbe => "absent-on-cbe",
            Self::Unassigned => "unassigned",
        }
    }
}

/// One instruction word, its opcode-map row, and its class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuWord {
    /// The word.
    pub raw: u32,
    /// The index and row of the opcode map the word selects, or `None`.
    pub row: Option<(usize, &'static SpuOpcodeRow)>,
    /// How the decoder treats the word.
    pub class: SpuWordClass,
}

impl SpuWord {
    /// Classify `raw`.
    pub fn of(raw: u32) -> Self {
        let class = match crate::decode::decode(raw) {
            Ok(_) => SpuWordClass::Implemented,
            Err(SpuDecodeError::Unimplemented { .. }) => SpuWordClass::NotImplemented,
            Err(SpuDecodeError::AbsentOnCbe { .. }) => SpuWordClass::AbsentOnCbe,
            Err(SpuDecodeError::Unassigned(_)) => SpuWordClass::Unassigned,
        };
        Self {
            raw,
            row: spu_isa::row_for(raw),
            class,
        }
    }
}

impl SpuWord {
    /// The bits the text shows as operand fields: every field position of
    /// the word's form, or none for a word no row selects.
    pub fn rendered_field_mask(&self) -> u32 {
        let Some((_, row)) = self.row else {
            return 0;
        };
        match row.form {
            SpuForm::Rr | SpuForm::Ri7 => 0x001f_ffff,
            SpuForm::Rrr => 0x0fff_ffff,
            SpuForm::Ri8 => 0x003f_ffff,
            SpuForm::Ri10 => 0x00ff_ffff,
            SpuForm::Ri16 => 0x007f_ffff,
            SpuForm::Ri18 => 0x01ff_ffff,
            SpuForm::Hint => 0x01ff_ffff,
        }
    }
}

/// Whether the text of `raw` agrees with the decoder: the word renders,
/// and a word that decodes names the mnemonic of the instruction the
/// decoder builds.
pub fn agrees_with_decode(raw: u32) -> bool {
    let word = SpuWord::of(raw);
    // Rendering to a sink that keeps nothing still runs every field read.
    let _ = fmt::write(&mut Discard, format_args!("{word}"));
    match crate::decode::decode(raw) {
        Ok(instruction) => {
            let kind: &'static str =
                crate::instruction::SpuInstructionKind::from(instruction).into();
            word.row
                .is_some_and(|(_, row)| kind.eq_ignore_ascii_case(row.mnemonic))
        }
        Err(_) => true,
    }
}

/// A text sink that keeps nothing.
struct Discard;

impl fmt::Write for Discard {
    fn write_str(&mut self, _: &str) -> fmt::Result {
        Ok(())
    }
}

impl fmt::Display for SpuWord {
    /// `mnemonic operands`, or `.word 0x...` for a word no row selects.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some((_, row)) = self.row else {
            return write!(f, ".word 0x{:08x}", self.raw);
        };
        let raw = self.raw;
        let field = |shift: u32, bits: u32| (raw >> shift) & ((1 << bits) - 1);
        let rt = field(0, 7);
        let ra = field(7, 7);
        let rb = field(14, 7);
        write!(f, "{:<9}", row.mnemonic)?;
        match row.form {
            SpuForm::Rr => write!(f, "${rt},${ra},${rb}"),
            SpuForm::Rrr => write!(f, "${},${ra},${rb},${rt}", field(21, 7)),
            SpuForm::Ri7 => write!(f, "${rt},${ra},0x{:x}", field(14, 7)),
            SpuForm::Ri8 => write!(f, "${rt},${ra},0x{:x}", field(14, 8)),
            SpuForm::Ri10 => write!(f, "${rt},${ra},0x{:x}", field(14, 10)),
            SpuForm::Ri16 => write!(f, "${rt},0x{:x}", field(7, 16)),
            SpuForm::Ri18 => write!(f, "${rt},0x{:x}", field(7, 18)),
            SpuForm::Hint => write!(f, "0x{:x},0x{:x}", (field(23, 2) << 7) | rt, field(7, 16)),
        }
    }
}

#[cfg(test)]
#[path = "tests/disasm_tests.rs"]
mod tests;
