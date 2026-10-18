//! The places the documents leave a result open, and the vector classes
//! that reach them.

use serde::Serialize;

/// One open question: a component of a result that no document fixes.
///
/// [SPU-ISA p:195 s:9] the architecture lets implementations produce different floating-point results; matching one takes more than compliance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Question {
    /// The frest and frsqest base and step values.
    EstimateTables,
    /// fi with an RB exponent of 0, and the DIFF flag for an RA whose
    /// exponent field reads as a denormal or as exponent 255.
    FiOperandFields,
    /// OVF for an exact single-precision result in (Smax, 2^129).
    SingleOverflowBand,
    /// A conversion whose scale falls outside 0 to 127.
    ConversionScale,
    /// A double-precision operation with a denormal operand.
    DoubleDenormalOperand,
    /// The payload and sign of a QNaN result from a NaN operand, and the
    /// NaN flag.
    DoubleNanResult,
    /// INV for infinity times zero plus a QNaN in the fused double forms.
    FusedInfinityZeroQnan,
    /// frds and fesd of a denormal input.
    ConversionDenormalInput,
}

impl Question {
    /// Every question, in a fixed order.
    pub const ALL: [Self; 8] = [
        Self::EstimateTables,
        Self::FiOperandFields,
        Self::SingleOverflowBand,
        Self::ConversionScale,
        Self::DoubleDenormalOperand,
        Self::DoubleNanResult,
        Self::FusedInfinityZeroQnan,
        Self::ConversionDenormalInput,
    ];

    /// The stable name the vector list and ADR 0009 use.
    pub fn id(self) -> &'static str {
        match self {
            Self::EstimateTables => "estimate-tables",
            Self::FiOperandFields => "fi-operand-fields",
            Self::SingleOverflowBand => "single-overflow-band",
            Self::ConversionScale => "conversion-scale",
            Self::DoubleDenormalOperand => "double-denormal-operand",
            Self::DoubleNanResult => "double-nan-result",
            Self::FusedInfinityZeroQnan => "fused-infinity-zero-qnan",
            Self::ConversionDenormalInput => "conversion-denormal-input",
        }
    }

    /// The page that leaves the component open.
    pub fn citation(self) -> &'static str {
        match self {
            // [SPU-ISA p:215 s:9] frest computes a base and a step; no page gives the values.
            Self::EstimateTables => "SPU-ISA p:215 s:9",
            // [SPU-ISA p:219 s:9] fi disassembles RB into a base and a step and reads RA[13:31] as a fraction.
            Self::FiOperandFields => "SPU-ISA p:219 s:9",
            // [SPU-ISA p:196 s:9.1] OVF when the result before rounding is bigger than Smax.
            Self::SingleOverflowBand => "SPU-ISA p:196 s:9.1",
            // [SPU-ISA p:220 s:9] a scale outside 0 to 127 makes the result undefined.
            Self::ConversionScale => "SPU-ISA p:220 s:9",
            // [SPU-ISA p:198 s:9.2] some implementations treat denormal operands as zeros.
            Self::DoubleDenormalOperand => "SPU-ISA p:198 s:9.2",
            // [SPU-ISA p:197 s:9.2] with a NaN input the result is the default QNaN or one of the input NaNs.
            Self::DoubleNanResult => "SPU-ISA p:197 s:9.2",
            // [SPU-ISA p:199 s:9.2.2] infinity times zero is invalid; no case names a NaN addend.
            Self::FusedInfinityZeroQnan => "SPU-ISA p:199 s:9.2.2",
            // [SPU-ISA p:198 s:9.2.1] some implementations force a denormal input to zero.
            Self::ConversionDenormalInput => "SPU-ISA p:198 s:9.2.1",
        }
    }

    /// The prefixes of the ids of every vector that reaches the question.
    pub fn vector_prefixes(self) -> &'static [&'static str] {
        const DOUBLE: &[&str] = &["dfa/", "dfs/", "dfm/", "dfma/", "dfms/", "dfnms/", "dfnma/"];
        match self {
            Self::EstimateTables => &["frest/", "frsqest/"],
            Self::FiOperandFields => &["fi/"],
            Self::SingleOverflowBand => &["fa/", "fs/", "fm/", "fma/", "fms/", "fnms/"],
            Self::ConversionScale => &[
                "csflt/undefined-i8/",
                "cuflt/undefined-i8/",
                "cflts/undefined-i8/",
                "cfltu/undefined-i8/",
            ],
            Self::DoubleDenormalOperand => DOUBLE,
            Self::DoubleNanResult => &[
                "dfa/", "dfs/", "dfm/", "dfma/", "dfms/", "dfnms/", "dfnma/", "frds/", "fesd/",
            ],
            Self::FusedInfinityZeroQnan => &["dfma/", "dfms/", "dfnms/", "dfnma/"],
            Self::ConversionDenormalInput => &["frds/", "fesd/"],
        }
    }

    /// What the documents leave open, in one sentence.
    pub fn reason(self) -> &'static str {
        match self {
            Self::EstimateTables => {
                "no document gives the frest or frsqest table or exponent map, only the result format"
            }
            Self::FiOperandFields => {
                "RB with exponent 0 is not a base and step the formula covers, and RA is a fraction, \
                 so whether its exponent field sets DIFF is not stated"
            }
            Self::SingleOverflowBand => {
                "the ISA sets OVF above Smax before rounding, and the exponent rounder of the \
                 hardware papers only above exponent 255"
            }
            Self::ConversionScale => "the ISA calls a scale outside 0 to 127 undefined",
            Self::DoubleDenormalOperand => {
                "an implementation may compute with a denormal operand or force it to a signed zero \
                 with DENORM, with or without INV"
            }
            Self::DoubleNanResult => {
                "an implementation may return the default QNaN or an input NaN, and sets the NaN \
                 flag only for the first"
            }
            Self::FusedInfinityZeroQnan => {
                "infinity times zero is invalid, but no page says whether a QNaN addend still \
                 raises INV"
            }
            Self::ConversionDenormalInput => {
                "an implementation may convert a denormal input or force it to zero with DENORM \
                 in place of UNF, with or without INV"
            }
        }
    }
}

/// The classes of vectors in the floating-point list, one per family of
/// instructions it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VectorClass {
    /// frest and frsqest.
    Estimate,
    /// fi.
    Interpolate,
    /// fa, fs, fm, fma, fms and fnms.
    SingleArithmetic,
    /// csflt, cuflt, cflts and cfltu.
    Conversion,
    /// dfa, dfs, dfm and the fused double forms.
    DoubleArithmetic,
    /// frds and fesd.
    RoundExtend,
    /// Two instructions whose slots and flags interact.
    Sequence,
}

impl VectorClass {
    /// Every class, in list order.
    pub const ALL: [Self; 7] = [
        Self::Estimate,
        Self::Interpolate,
        Self::SingleArithmetic,
        Self::Conversion,
        Self::DoubleArithmetic,
        Self::RoundExtend,
        Self::Sequence,
    ];
}
