//! The catalog text of each row: its register names, what a fused partner
//! computes and which start states the precondition admits; and each row
//! word as SPU assembly with the ISA page that defines it.

use cellgov_ps3_abi::hw::spu_isa::{self, SpuOpcodeRow};

use crate::instruction::SpuInstructionKind;

use super::super::classify::form_for_kind;
use super::super::metamorphic::opcode_word;
use super::super::relations::ignored_field_mask;
use super::super::types::SpuEncodingForm;
use super::types::{SpuSequenceRelationId, SpuSymbolicWord};

/// The label that names a relative branch's target, the taken landing.
const TAKEN_LABEL: &str = "taken";

/// The RB field of an RR word.
///
/// [SPU-ISA p:28 s:2.3] RB is bits 11:17, which are bits 14..=20 from the
/// low end.
const RB_FIELD: u32 = 0x7F << 14;

/// The first page of each SPU ISA chapter that defines instructions, and
/// the chapter's number.
const CHAPTERS: [(u16, u8); 10] = [
    (31, 3),
    (49, 4),
    (57, 5),
    (117, 6),
    (149, 7),
    (191, 8),
    (195, 9),
    (237, 10),
    (247, 11),
    (251, 12),
];

/// The catalog text of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuSequenceRelationText {
    /// The name of each symbolic register, in symbolic order.
    pub registers: &'static [&'static str],
    /// What the fused partner computes; `None` for a guest partner.
    pub fused: Option<&'static str>,
    /// The start states the precondition admits; `None` for a row that
    /// covers every start state.
    pub precondition: Option<&'static str>,
}

const NOT_EQUAL: &[&str] = &["c", "a", "b", "rt"];
const MPY32: &[&str] = &["t1", "a", "b", "t2", "s", "u", "rt"];
const SELECT: &[&str] = &["c", "x", "y", "rt", "a", "b"];
const SELECT_IMMEDIATE: &[&str] = &["c", "x", "rt", "a", "b"];
const SPLAT: &[&str] = &["c", "x", "y", "rt"];
const INSERT_D: &[&str] = &["m", "p", "rt", "a", "b"];
const INSERT_X: &[&str] = &["m", "p", "q", "rt", "a", "b"];
const NEGATED: &[&str] = &["n", "x", "rt", "a"];
const FUNNEL: &[&str] = &["v", "x", "s", "rt"];
const BRANCH: &[&str] = &["o", "v"];
const BRANCH_INDIRECT: &[&str] = &["o", "v", "t"];
const POPCOUNT: &[&str] = &["c", "a", "rt"];
const SPLIT: &[&str] = &["x", "y", "r"];
const MOVE: &[&str] = &["m", "x", "rt", "y"];
const ESTIMATE: &[&str] = &["t", "x", "rt"];
const NEWTON: &[&str] = &["y0", "d", "y", "e", "one", "rt"];
const RSQRT_NEWTON: &[&str] = &[
    "ax", "x", "mask", "y0", "y1", "t1", "t2", "half", "t3", "one", "rt",
];
const SQUARE_ROOT: &[&str] = &["y0", "x", "y", "g", "h", "half", "t", "one", "rt"];
const DIVISION: &[&str] = &["y0", "b", "y", "q", "a", "t", "rt"];
const PICK: &[&str] = &["c", "a", "b", "rt"];

const SELECT_FUSED: &str =
    "c = the compare's lane mask; each lane of rt = b where the mask is set, a elsewhere";
const SELECT_PRECONDITION: &str = "c is neither a nor b";
const SPLAT_FUSED: &str = "c = the word compare's lane mask; every word of rt = its preferred word";
const INSERT_D_FUSED: &str = "m = the shuffle control; rt = b with the preferred element of a \
     inserted at byte (p + 5), aligned down to the element size";
const INSERT_X_FUSED: &str = "m = the shuffle control; rt = b with the preferred element of a \
     inserted at byte (p + q), aligned down to the element size";
const NEGATED_PRECONDITION: &str = "n is not a";
const BRANCH_ZERO: &str = "o = the OR of the four words of v in its preferred word, zero in the \
     others; control goes to the taken landing when the OR is zero";
const BRANCH_NOT_ZERO: &str = "o = the OR of the four words of v in its preferred word, zero in \
     the others; control goes to the taken landing when the OR is not zero";
const BRANCH_INDIRECT_PRECONDITION: &str = "o is not t";
const NEWTON_FUSED: &str = "rt = the truncated reciprocal of d, per word";
const NEWTON_PRECONDITION: &str =
    "every symbolic register names its own register; every word of d has an exponent in 1..=252";
const PICK_PRECONDITION: &str = "c, a and b are three registers; no word of a or b has exponent \
     255, and no lane pairs two zero exponents";

impl SpuSequenceRelationId {
    /// The row's catalog text.
    pub fn text(self) -> SpuSequenceRelationText {
        use SpuSequenceRelationId as Id;
        let fused = |registers, fused, precondition| SpuSequenceRelationText {
            registers,
            fused: Some(fused),
            precondition,
        };
        let guest = |registers, precondition| SpuSequenceRelationText {
            registers,
            fused: None,
            precondition,
        };
        match self {
            Id::CeqNotEqualFused => fused(
                NOT_EQUAL,
                "c = the word compare a == b; rt = its complement, sext(a != b)",
                None,
            ),
            Id::CeqNotEqualNor => guest(NOT_EQUAL, None),
            Id::CeqNotEqualResultOnly => fused(
                NOT_EQUAL,
                "rt = sext(a != b) per word; c keeps its start value",
                None,
            ),
            Id::CeqhNotEqualFused => fused(
                NOT_EQUAL,
                "c = the halfword compare a == b; rt = its complement, sext(a != b) per halfword",
                None,
            ),
            Id::Mpy32 | Id::Mpy32Swapped => fused(
                MPY32,
                "rt = the low 32 bits of a * b, per word; t1, t2, s and u = the sequence's \
                 intermediates",
                Some("t1, t2, s and u are four registers, and none of them is a or b"),
            ),
            Id::SelectCeq
            | Id::SelectCeqh
            | Id::SelectCeqb
            | Id::SelectCgt
            | Id::SelectCgth
            | Id::SelectCgtb
            | Id::SelectClgt
            | Id::SelectClgth
            | Id::SelectClgtb
            | Id::SelectFceq
            | Id::SelectFcgt
            | Id::SelectFcmeq
            | Id::SelectFcmgt => fused(SELECT, SELECT_FUSED, Some(SELECT_PRECONDITION)),
            Id::SelectCeqi
            | Id::SelectCeqhi
            | Id::SelectCeqbi
            | Id::SelectCgti
            | Id::SelectCgthi
            | Id::SelectCgtbi
            | Id::SelectClgti
            | Id::SelectClgthi
            | Id::SelectClgtbi => fused(SELECT_IMMEDIATE, SELECT_FUSED, Some(SELECT_PRECONDITION)),
            Id::SplatCeq | Id::SplatCgt | Id::SplatClgt => fused(SPLAT, SPLAT_FUSED, None),
            Id::InsertCbd | Id::InsertChd | Id::InsertCwd | Id::InsertCdd => {
                fused(INSERT_D, INSERT_D_FUSED, None)
            }
            Id::InsertCbx | Id::InsertChx | Id::InsertCwx | Id::InsertCdx => {
                fused(INSERT_X, INSERT_X_FUSED, None)
            }
            Id::NegatedCountRotm => fused(
                NEGATED,
                "n = 0 - x per word; rt = a shifted right logically by x & 0x3F per word, zero \
                 at 32 or more",
                Some(NEGATED_PRECONDITION),
            ),
            Id::NegatedCountRotma => fused(
                NEGATED,
                "n = 0 - x per word; rt = a shifted right arithmetically by x & 0x3F per word, \
                 the sign at 32 or more",
                Some(NEGATED_PRECONDITION),
            ),
            Id::NegatedCountRothm => fused(
                NEGATED,
                "n = 0 - x per halfword; rt = a shifted right logically by x & 0x1F per \
                 halfword, zero at 16 or more",
                Some(NEGATED_PRECONDITION),
            ),
            Id::NegatedCountRotmah => fused(
                NEGATED,
                "n = 0 - x per halfword; rt = a shifted right arithmetically by x & 0x1F per \
                 halfword, the sign at 16 or more",
                Some(NEGATED_PRECONDITION),
            ),
            Id::NegatedCountRotqmbi => fused(
                NEGATED,
                "n = 0 - x per word; rt = the quadword a shifted right by the preferred word of \
                 x & 7 bits",
                Some(NEGATED_PRECONDITION),
            ),
            Id::NegatedCountRotqmby => fused(
                NEGATED,
                "n = 0 - x per word; rt = the quadword a shifted right by the preferred word of \
                 x & 0x1F bytes, zero at 16 or more",
                Some(NEGATED_PRECONDITION),
            ),
            Id::FunnelShift => fused(
                FUNNEL,
                "v = the quadword x rotated left by (s >> 3) & 0xF bytes; rt = x rotated left by \
                 s & 0x7F bits; s is read from its preferred word",
                Some("v is not s"),
            ),
            Id::BranchOrxBrz => fused(BRANCH, BRANCH_ZERO, None),
            Id::BranchOrxBrnz => fused(BRANCH, BRANCH_NOT_ZERO, None),
            Id::BranchOrxBiz => fused(
                BRANCH_INDIRECT,
                BRANCH_ZERO,
                Some(BRANCH_INDIRECT_PRECONDITION),
            ),
            Id::BranchOrxBinz => fused(
                BRANCH_INDIRECT,
                BRANCH_NOT_ZERO,
                Some(BRANCH_INDIRECT_PRECONDITION),
            ),
            Id::Popcount => fused(
                POPCOUNT,
                "c = the count of one bits in each byte of a; each word of rt = the count of one \
                 bits in its word of a, in both halfwords",
                None,
            ),
            Id::SplitAddressLoad | Id::SplitAddressStore => guest(
                SPLIT,
                Some(
                    "x is not y, and the accessed quadword lies outside the program and the \
                     taken landing",
                ),
            ),
            Id::MoveOriAi | Id::MoveOriAndi | Id::MoveOriShlqbyi => guest(MOVE, None),
            Id::EstimateReciprocal => fused(
                ESTIMATE,
                "rt = the truncated reciprocal of x, per word",
                Some(
                    "every symbolic register names its own register; every word of x has an \
                     exponent in 1..=252",
                ),
            ),
            Id::EstimateRsqrt => fused(
                ESTIMATE,
                "rt = the truncated reciprocal square root of |x|, per word",
                Some(
                    "every symbolic register names its own register; every word of x has an \
                     exponent in 1..=254",
                ),
            ),
            Id::NewtonReciprocal | Id::NewtonReciprocalOnePlus => {
                fused(NEWTON, NEWTON_FUSED, Some(NEWTON_PRECONDITION))
            }
            Id::RsqrtNewton => fused(
                RSQRT_NEWTON,
                "rt = the truncated reciprocal square root of |x|, per word",
                Some(
                    "every symbolic register names its own register; every word of x has an \
                     exponent in 1..=254",
                ),
            ),
            Id::SquareRoot => fused(
                SQUARE_ROOT,
                "rt = the IEEE single-precision sqrt(|x|), rounded to nearest, per word",
                Some(
                    "every symbolic register names its own register; every word of x is \
                     positive with an exponent in 1..=254",
                ),
            ),
            Id::Division => fused(
                DIVISION,
                "rt = the IEEE single-precision a / b, rounded to nearest, per word",
                Some(
                    "every symbolic register names its own register; in each word, the \
                     exponent of a is in 50..=254, that of b in 1..=252, and the exponent of a \
                     plus 127 less that of b in 2..=253",
                ),
            ),
            Id::FloatMax => fused(
                PICK,
                "rt = the IEEE maximum of a and b per word: the other operand where one is a \
                 NaN",
                Some(PICK_PRECONDITION),
            ),
            Id::FloatMin => fused(
                PICK,
                "rt = the IEEE minimum of a and b per word: the other operand where one is a \
                 NaN",
                Some(PICK_PRECONDITION),
            ),
            Id::MagnitudeMax => fused(
                PICK,
                "rt = a where |a| > |b| in IEEE order, b elsewhere, per word",
                Some(PICK_PRECONDITION),
            ),
            Id::MagnitudeMin => fused(
                PICK,
                "rt = b where |a| > |b| in IEEE order, a elsewhere, per word",
                Some(PICK_PRECONDITION),
            ),
            Id::EqualPick => fused(
                PICK,
                "rt = a where a equals b in IEEE order, or both are zeros, b elsewhere, per word",
                Some(PICK_PRECONDITION),
            ),
            Id::FloatMaxSelect => fused(
                PICK,
                "c = the fcgt lane mask of a > b; rt = a where the mask is set, b elsewhere",
                Some("c is neither a nor b"),
            ),
        }
    }
}

/// The opcode-map row of `kind`.
fn opcode_row(kind: SpuInstructionKind) -> Option<&'static SpuOpcodeRow> {
    spu_isa::row_for(opcode_word(kind)?).map(|(_, row)| row)
}

/// `value`'s low `bits` bits as a signed number.
fn sign_extend(value: u32, bits: u32) -> i32 {
    ((value << (32 - bits)) as i32) >> (32 - bits)
}

impl SpuSymbolicWord {
    /// The word in SPU assembly: each symbolic register by its name in
    /// `names`, and a relative branch's target as `taken`. `None`
    /// for an encoding form the rows do not use, or an index past `names`.
    pub fn assembly(&self, names: &[&str]) -> Option<String> {
        use SpuInstructionKind as K;
        let name = |index: u8| names.get(usize::from(index)).copied();
        let mnemonic = opcode_row(self.kind)?.mnemonic;
        let rt = name(self.rt)?;
        Some(match form_for_kind(self.kind) {
            SpuEncodingForm::Rrr => {
                let ra = name(self.ra)?;
                if ignored_field_mask(self.kind).is_some_and(|mask| mask & RB_FIELD == RB_FIELD) {
                    format!("{mnemonic} {rt},{ra}")
                } else {
                    format!("{mnemonic} {rt},{ra},{}", name(self.rb)?)
                }
            }
            SpuEncodingForm::Rrrr => format!(
                "{mnemonic} {rt},{},{},{}",
                name(self.ra)?,
                name(self.rb)?,
                name(self.rc)?
            ),
            SpuEncodingForm::Ri10 => {
                let imm = sign_extend(self.imm, 10);
                match self.kind {
                    // [SPU-ISA p:32 s:3 Lqd] the address is I10 shifted left
                    // 4, plus RA.
                    K::Lqd | K::Stqd => format!("{mnemonic} {rt},{}({})", imm * 16, name(self.ra)?),
                    _ => format!("{mnemonic} {rt},{},{imm}", name(self.ra)?),
                }
            }
            SpuEncodingForm::Ri7 => {
                let imm = sign_extend(self.imm, 7);
                match self.kind {
                    K::Cbd | K::Chd | K::Cwd | K::Cdd => {
                        format!("{mnemonic} {rt},{imm}({})", name(self.ra)?)
                    }
                    _ => format!("{mnemonic} {rt},{},{imm}", name(self.ra)?),
                }
            }
            SpuEncodingForm::Branch => match self.kind {
                K::Brz | K::Brnz | K::Brhz | K::Brhnz => format!("{mnemonic} {rt},{TAKEN_LABEL}"),
                K::Biz | K::Binz | K::Bihz | K::Bihnz => {
                    format!("{mnemonic} {rt},{}", name(self.ra)?)
                }
                _ => return None,
            },
            _ => return None,
        })
    }
}

/// The SPU ISA citation of the page that defines `kind`, in the
/// `DOC-KEY p:PAGE s:SECTION` form: the section is the chapter and the
/// mnemonic.
///
/// [SPU-ISA p:259 s:A] Table A-1 gives the page of each instruction.
pub fn isa_citation(kind: SpuInstructionKind) -> Option<String> {
    let row = opcode_row(kind)?;
    let chapter = CHAPTERS
        .iter()
        .rev()
        .find(|(first, _)| row.page >= *first)?
        .1;
    let mut name = row.mnemonic.to_owned();
    name.get_mut(..1)?.make_ascii_uppercase();
    let page = format!("p:{}", row.page);
    Some(format!("[{} {page} s:{chapter} {name}]", "SPU-ISA"))
}
