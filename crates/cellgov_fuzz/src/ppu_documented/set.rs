//! The sets of the PPU list: each one reaches a place the documents
//! leave a result open.

use serde::Serialize;

/// One set of vectors, named as the consumers of its answers name it.
///
/// [PPC-Book1 p:3 s:1.5.1] an undefined value may vary between implementations and between executions on the same implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Set {
    /// The high word of the word multiplies and divides, and the
    /// divides the documents leave undefined.
    MuldivUndef,
    /// The CR field moves with a mask of other than one field, `lswx`
    /// with a zero count, and the XER reserved bits.
    MoveUndef,
    /// The high word and FPRF after `fctiw` and `fctiwz`.
    FctiwHighWord,
    /// The high word after `mffs`.
    MffsHighWord,
    /// Single-precision arithmetic on operands single format cannot
    /// hold.
    Fp59WideOperand,
    /// FR after an overflow with overflow exceptions disabled.
    FpOverflowFr,
    /// The word `stfs` stores for a value below the single denormal
    /// range.
    StfsBelowDenormal,
    /// Floating-point results with non-IEEE mode on.
    FpscrNi,
    /// A store conditional to an address other than the reserved one.
    StwcxOtherAddress,
    /// Whether `dcbf` by the reserving thread clears its reservation.
    ReservationUndefined,
    /// The lanes the element loads do not load.
    LveOtherLanes,
    /// `vsl` and `vsr` when the shift counts of the bytes differ.
    VshiftNonuniform,
}

impl Set {
    /// Every set, in list order.
    pub const ALL: [Self; 12] = [
        Self::MuldivUndef,
        Self::MoveUndef,
        Self::FctiwHighWord,
        Self::MffsHighWord,
        Self::Fp59WideOperand,
        Self::FpOverflowFr,
        Self::StfsBelowDenormal,
        Self::FpscrNi,
        Self::StwcxOtherAddress,
        Self::ReservationUndefined,
        Self::LveOtherLanes,
        Self::VshiftNonuniform,
    ];

    /// The stable name the vector ids and the consumers use.
    pub fn id(self) -> &'static str {
        match self {
            Self::MuldivUndef => "muldiv-undef",
            Self::MoveUndef => "move-undef",
            Self::FctiwHighWord => "fctiw-high-word",
            Self::MffsHighWord => "mffs-high-word",
            Self::Fp59WideOperand => "fp59-wide-operand",
            Self::FpOverflowFr => "fp-overflow-fr",
            Self::StfsBelowDenormal => "stfs-below-denormal",
            Self::FpscrNi => "fpscr-ni",
            Self::StwcxOtherAddress => "stwcx-other-address",
            Self::ReservationUndefined => "reservation-undefined",
            Self::LveOtherLanes => "lve-other-lanes",
            Self::VshiftNonuniform => "vshift-nonuniform",
        }
    }

    /// What the documents leave open, in one sentence.
    pub fn reason(self) -> &'static str {
        match self {
            Self::MuldivUndef => {
                "the word forms leave RT[0:31] undefined, and a divide by zero or of the most \
                 negative value by -1 leaves RT and CR0 LT, GT and EQ undefined"
            }
            Self::MoveUndef => {
                "a CR field move whose mask selects other than one field leaves its target \
                 undefined, lswx with a zero count leaves RT undefined, and a reserved XER bit \
                 last written 1 reads back undefined"
            }
            Self::FctiwHighWord => "the conversion leaves FRT[0:31] and FPRF undefined",
            Self::MffsHighWord => "mffs leaves FRT[0:31] undefined",
            Self::Fp59WideOperand => {
                "a single-precision operation on an operand single format cannot hold leaves \
                 the result and the status bits undefined"
            }
            Self::FpOverflowFr => "an overflow with OE=0 leaves FR undefined",
            Self::StfsBelowDenormal => {
                "stfs of a nonzero value below the single denormal range stores an undefined word"
            }
            Self::FpscrNi => {
                "with NI=1 the results need not conform to IEEE and the FPSCR bits may mean \
                 other things"
            }
            Self::StwcxOtherAddress => {
                "a store conditional to an address other than the reserved one may or may not \
                 store, and CR0 EQ is undefined"
            }
            Self::ReservationUndefined => {
                "whether a dcbf to the reservation granule by the reserving thread clears the \
                 reservation is undefined"
            }
            Self::LveOtherLanes => "the element loads leave every lane they do not load undefined",
            Self::VshiftNonuniform => {
                "vsl and vsr give an undefined result when the low three bits of the bytes of vB \
                 differ"
            }
        }
    }
}
