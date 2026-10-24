//! The two-instruction sequences.

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::builder::{from_doublewords, from_words, Builder, SplitMix};
use super::classes::{DoubleClass, SingleClass};
use super::conversion::{TO_FLOAT_SCALES, TO_INTEGER_SCALES};
use crate::spu_documented::dyadic::Mode;
use crate::spu_documented::model::{with_mode, Insn, Op};
use crate::spu_documented::question::VectorClass;

/// The sequence kinds, by name.
pub(crate) const SEQUENCE_KINDS: [&str; 5] = [
    "single-chain",
    "double-chain",
    "mixed-slices",
    "sticky",
    "convert-chain",
];

/// The two-instruction sequences.
///
/// In each sequence one of these holds:
/// - a result feeds the next instruction;
/// - the two instructions raise flags in different slices;
/// - both instructions raise the same sticky flag.
///
/// [SPU-ISA p:200 s:9.3] the status bits are sticky and each slice has its own fields.
pub(super) fn sequences(b: &mut Builder) {
    for kind in SEQUENCE_KINDS {
        let mut made = 0;
        let mut attempts = 0;
        while made < 20 && attempts < 400 {
            attempts += 1;
            let r = &mut b.rng;
            let (insns, registers) = draw_sequence(kind, r);
            let mut fpscr = (u128::from(r.next()) << 64 | u128::from(r.next())) & FPSCR_DEFINED;
            if r.bit() {
                // Half the sequences start with no sticky flag.
                fpscr &= !FPSCR_DEFINED | with_mode(0, 0, Mode::Down) | with_mode(0, 1, Mode::Down);
            }
            let id = format!("seq/{kind}/{made:04}");
            let cells = vec![format!("seq:{kind}")];
            if b.push(
                id,
                VectorClass::Sequence,
                cells,
                &insns,
                registers.into_iter().collect(),
                fpscr,
            ) {
                made += 1;
            }
        }
    }
}

/// The two instructions of a `kind` sequence and the input registers
/// they read, drawn from `r`.
fn draw_sequence(kind: &str, r: &mut SplitMix) -> (Vec<Insn>, Vec<(u8, u128)>) {
    const NORMAL: [SingleClass; 3] = [SingleClass::Low, SingleClass::Mid, SingleClass::High];
    let single = |r: &mut SplitMix| {
        from_words(std::array::from_fn(|_| {
            NORMAL[r.below(3) as usize].draw(r) | u32::from(r.bit()) << 31
        }))
    };
    let double = |r: &mut SplitMix| {
        from_doublewords(std::array::from_fn(|_| {
            let class =
                [DoubleClass::Small, DoubleClass::Mid, DoubleClass::Large][r.below(3) as usize];
            class.draw(r) | u64::from(r.bit()) << 63
        }))
    };
    match kind {
        "single-chain" => (
            vec![Insn::new(Op::Fa, 3, 4, 5), Insn::new(Op::Fm, 7, 3, 6)],
            vec![(4, single(r)), (5, single(r)), (6, single(r))],
        ),
        "double-chain" => (
            vec![Insn::new(Op::Dfa, 3, 4, 5), Insn::new(Op::Frds, 7, 3, 0)],
            vec![(4, double(r)), (5, double(r))],
        ),
        "mixed-slices" => (
            vec![Insn::new(Op::Fm, 3, 4, 5), Insn::new(Op::Dfm, 7, 8, 9)],
            vec![
                (4, single(r)),
                (5, single(r)),
                (8, double(r)),
                (9, double(r)),
            ],
        ),
        "sticky" => (
            vec![Insn::new(Op::Fa, 3, 4, 5), Insn::new(Op::Fa, 3, 3, 6)],
            vec![(4, single(r)), (5, single(r)), (6, single(r))],
        ),
        _ => {
            let first = TO_FLOAT_SCALE_BIAS - TO_FLOAT_SCALES[r.below(11) as usize];
            let second = TO_INTEGER_SCALE_BIAS - TO_INTEGER_SCALES[r.below(10) as usize];
            (
                vec![
                    Insn::scaled(Op::Csflt, 3, 4, first),
                    Insn::scaled(Op::Cflts, 7, 3, second),
                ],
                vec![(4, from_words(std::array::from_fn(|_| r.next() as u32)))],
            )
        }
    }
}
