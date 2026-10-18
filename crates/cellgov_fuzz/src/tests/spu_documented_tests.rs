use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use super::dyadic::{round, Dyadic, Mode, DOUBLE, SINGLE};
use super::generate::{
    double_cell, mode_name, signed_name, single_cell, DOUBLE_CLASSES, FESD_CLASSES, FRDS_CLASSES,
    FUSED_CLASSES, SEQUENCE_KINDS, SINGLE_CLASSES, TO_FLOAT_SCALES, TO_INTEGER_SCALES,
};
use super::model::{
    double_slot, effect, extended_result, interpolate, round_to_single, to_integer, Insn, Known, Op,
};
use super::*;

/// The digest of the rendered list. A change to the generator or the
/// model moves it; every host renders the same list.
const GOLDEN: u64 = 0x48c7_3987_1364_b31e;

fn list() -> &'static [DocumentedVector] {
    static LIST: OnceLock<Vec<DocumentedVector>> = OnceLock::new();
    LIST.get_or_init(documented_vectors)
}

#[test]
fn the_rendered_list_matches_its_pinned_digest() {
    let text = render(list()).unwrap();
    assert_eq!(text.lines().count(), list().len());
    assert_eq!(digest(&text), GOLDEN, "{:#018x}", digest(&text));
}

#[test]
fn the_replay_against_the_documents_is_clean() {
    let replay = replay_documented(list());
    assert!(
        replay.is_clean(),
        "{:#?} {:#?}",
        &replay.stopped[..replay.stopped.len().min(20)],
        &replay.differences[..replay.differences.len().min(20)]
    );
    assert_eq!(replay.vectors, list().len());
    assert!(replay.documented > 100_000, "{}", replay.documented);
    for question in Question::ALL {
        assert!(
            replay.undefined.get(&question).copied().unwrap_or(0) > 0,
            "{question:?} skipped nothing"
        );
    }
}

#[test]
fn a_one_bit_change_to_a_documented_value_is_a_difference() {
    let mut vectors: Vec<DocumentedVector> = Vec::new();
    let mut changed = BTreeSet::new();
    for class in VectorClass::ALL {
        let mut vector = list().iter().find(|v| v.class == class).unwrap().clone();
        let component = vector
            .expected
            .iter_mut()
            .find(|c| matches!(c.verdict, Verdict::Value { .. }) && c.mask != 0)
            .unwrap();
        if let Verdict::Value { value, .. } = &mut component.verdict {
            *value ^= 1 << component.mask.trailing_zeros();
        }
        changed.insert((vector.id.clone(), component.name.clone()));
        vectors.push(vector);
    }
    let replay = replay_documented(&vectors);
    let differing: BTreeSet<_> = replay
        .differences
        .iter()
        .map(|d| (d.vector.clone(), d.component.clone()))
        .collect();
    assert_eq!(differing, changed);
}

#[test]
fn a_vector_whose_instruction_does_not_run_is_stopped_and_not_clean() {
    // Both end states equal the loaded state, which every documented
    // component of this vector matches: only the stop record shows them.
    let template = list()
        .iter()
        .find(|v| v.id == "cflts/undefined-i8/000")
        .unwrap();
    // Exponent 255 at scale 173 is 270 past the saturation point: a
    // refusal.
    let mut faulting = template.clone();
    for value in faulting.registers.values_mut() {
        *value = 0x7FFF_FFFF_7FFF_FFFF_7FFF_FFFF_7FFF_FFFF;
    }
    // Word 0 is stop, which does not continue.
    let mut stopping = template.clone();
    stopping.id = "stop".to_owned();
    stopping.words = vec![0];
    let replay = replay_documented(&[faulting, stopping]);
    let stopped: Vec<(&str, usize)> = replay
        .stopped
        .iter()
        .map(|s| (s.vector.as_str(), s.step))
        .collect();
    assert_eq!(stopped, [("cflts/undefined-i8/000", 0), ("stop", 0)]);
    assert_eq!((replay.documented, replay.differences.len()), (0, 0));
    assert!(!replay.is_clean());
}

#[test]
fn every_overflow_band_slot_has_its_exact_result_in_the_open_band() {
    let open = Verdict::Undefined {
        question: Question::SingleOverflowBand,
    };
    let mut checked = 0;
    for vector in list().iter().filter(|v| v.id.contains("/band/")) {
        for slot in 0..4 {
            let name = format!("fpscr.s{slot}.ovf");
            let ovf = vector.expected.iter().find(|c| c.name == name).unwrap();
            // A flag set before the instruction stays set: no question.
            let sticky = vector.fpscr & ovf.mask != 0;
            assert!(
                sticky || ovf.verdict == open,
                "{} {name}: {:?}",
                vector.id,
                ovf.verdict
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 6 * 16);
}

#[test]
fn every_exact_zero_slot_has_a_zero_result() {
    const SIGNS: u128 = 0x8000_0000_8000_0000_8000_0000_8000_0000;
    let mut checked = 0;
    for vector in list().iter().filter(|v| v.id.contains("/exact-zero/")) {
        for component in &vector.expected {
            if component.location != Location::Register(3) {
                continue;
            }
            let Verdict::Value { value, .. } = component.verdict else {
                panic!("{} {} is open", vector.id, component.name);
            };
            assert_eq!(
                value & component.mask & !SIGNS,
                0,
                "{} {}",
                vector.id,
                component.name
            );
            checked += 1;
        }
    }
    // Single: five forms, eight slots. Double: six forms, sixteen slots.
    assert_eq!(checked, 5 * 8 + 6 * 16);
}

#[test]
fn every_word_decodes_as_the_instruction_its_vector_names() {
    let mut checked = 0;
    for vector in list() {
        let mnemonic = vector.id.split('/').next().unwrap();
        for &word in &vector.words {
            let insn = cellgov_spu::decode::decode(word).unwrap();
            if vector.class != VectorClass::Sequence {
                let name = format!("{insn:?}");
                let variant = name.split([' ', '{']).next().unwrap().to_lowercase();
                assert_eq!(variant, mnemonic, "{} word {word:#010x}", vector.id);
                checked += 1;
            }
        }
    }
    assert!(checked > 10_000, "{checked}");
}

#[test]
fn every_component_cites_a_page_of_the_isa() {
    let valid = |citation: &str| {
        citation
            .strip_prefix("SPU-ISA p:")
            .and_then(|rest| rest.split_once(" s:"))
            .is_some_and(|(page, section)| {
                page.parse::<u32>().is_ok_and(|p| (1..=278).contains(&p)) && !section.is_empty()
            })
    };
    for question in Question::ALL {
        assert!(valid(question.citation()), "{question:?}");
    }
    let mut seen = 0;
    for vector in list() {
        for component in &vector.expected {
            if let Verdict::Value { citation, .. } = component.verdict {
                assert!(
                    valid(citation),
                    "{} {} {citation}",
                    vector.id,
                    component.name
                );
                seen += 1;
            }
        }
    }
    assert!(seen > 100_000);
}

#[test]
fn every_cell_of_the_coverage_model_has_a_vector() {
    let cells: BTreeSet<&str> = list()
        .iter()
        .flat_map(|v| v.cells.iter().map(String::as_str))
        .collect();
    // A fused double cell carries the one mode it was drawn with.
    let unmoded: BTreeSet<&str> = cells.iter().map(|c| c.split('@').next().unwrap()).collect();
    let mut expected: Vec<String> = Vec::new();
    let mut expected_unmoded: Vec<String> = Vec::new();
    let signed_single: Vec<_> = SINGLE_CLASSES
        .iter()
        .flat_map(|&c| [(c, false), (c, true)])
        .collect();
    for op in [Op::Fa, Op::Fs, Op::Fm] {
        for &x in &signed_single {
            for &y in &signed_single {
                expected.push(single_cell(op, &[x, y]));
            }
        }
    }
    for op in [Op::Fma, Op::Fms, Op::Fnms] {
        for &x in &signed_single {
            for &y in &signed_single {
                for &z in &signed_single {
                    expected.push(single_cell(op, &[x, y, z]));
                }
            }
        }
    }
    let signed_double: Vec<_> = DOUBLE_CLASSES
        .iter()
        .flat_map(|&c| [(c, false), (c, true)])
        .collect();
    for op in [Op::Dfa, Op::Dfs, Op::Dfm] {
        for &x in &signed_double {
            for &y in &signed_double {
                for mode in Mode::ALL {
                    expected.push(double_cell(op, &[x, y], mode));
                }
            }
        }
    }
    let signed_fused: Vec<_> = FUSED_CLASSES
        .iter()
        .flat_map(|&c| [(c, false), (c, true)])
        .collect();
    for op in [Op::Dfma, Op::Dfms, Op::Dfnms, Op::Dfnma] {
        for &x in &signed_fused {
            for &y in &signed_fused {
                for &z in &signed_fused {
                    let cell = double_cell(op, &[x, y, z], Mode::Nearest);
                    expected_unmoded.push(cell.split('@').next().unwrap().to_owned());
                }
            }
        }
    }
    for op in ["frest", "frsqest"] {
        expected.extend((0..32).map(|i| format!("{op}:index{i:02}")));
        expected.extend((0..=255).map(|e| format!("{op}:exponent{e:03}")));
    }
    expected.extend([0, 1, 2, 127, 254, 255].map(|e| format!("fi:edge-e{e:03}")));
    expected.extend(["fi:random", "fi:ra-exponent-zero", "fi:ra-exponent-255"].map(String::from));
    for (op, bias, scales) in [
        ("csflt", 155, &TO_FLOAT_SCALES[..]),
        ("cuflt", 155, &TO_FLOAT_SCALES[..]),
        ("cflts", 173, &TO_INTEGER_SCALES[..]),
        ("cfltu", 173, &TO_INTEGER_SCALES[..]),
    ] {
        expected.extend(scales.iter().map(|s| format!("{op}:scale{s:03}")));
        expected.extend(
            (0..=255i32)
                .filter(|imm| !(0..=127).contains(&(bias - imm)))
                .map(|imm| format!("{op}:undefined-i8-{imm:03}")),
        );
    }
    for op in ["fa", "fs", "fm", "fma", "fms", "fnms"] {
        expected.push(format!("{op}:overflow-band"));
        expected.push(format!("{op}:random"));
    }
    for op in ["fma", "fms", "fnms"] {
        expected.extend((-60..=60).map(|d| format!("{op}:exponent-difference{d:+03}")));
    }
    for op in ["fa", "fs", "fma", "fms", "fnms"] {
        expected.push(format!("{op}:exact-zero"));
    }
    for mode in Mode::ALL {
        for op in ["dfa", "dfs", "dfm", "dfma", "dfnms"] {
            expected.push(format!("{op}:underflow@{}", mode_name(mode)));
        }
        for op in ["dfa", "dfs", "dfma", "dfms", "dfnms", "dfnma"] {
            expected.push(format!("{op}:exact-zero@{}", mode_name(mode)));
        }
        for class in FRDS_CLASSES {
            for negative in [false, true] {
                expected.push(format!(
                    "frds:{}@{}",
                    signed_name(class, negative),
                    mode_name(mode)
                ));
            }
        }
    }
    for class in FESD_CLASSES {
        for negative in [false, true] {
            expected.push(format!("fesd:{}", signed_name(class, negative)));
        }
    }
    expected.extend(SEQUENCE_KINDS.map(|k| format!("seq:{k}")));
    let missing: Vec<&String> = expected
        .iter()
        .filter(|c| !cells.contains(c.as_str()))
        .collect();
    let missing_unmoded: Vec<&String> = expected_unmoded
        .iter()
        .filter(|c| !unmoded.contains(c.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "{} missing, first {:?}",
        missing.len(),
        &missing[..missing.len().min(5)]
    );
    assert!(
        missing_unmoded.is_empty(),
        "{} missing, first {:?}",
        missing_unmoded.len(),
        &missing_unmoded[..missing_unmoded.len().min(5)]
    );
    assert!(
        expected.len() + expected_unmoded.len() > 35_000,
        "{}",
        expected.len()
    );
    for class in VectorClass::ALL {
        assert!(list().iter().any(|v| v.class == class), "{class:?}");
    }
}

#[test]
fn every_question_names_the_vectors_that_reach_it() {
    let mut reached = 0;
    for question in Question::ALL {
        let prefixes = question.vector_prefixes();
        let reaching: Vec<&DocumentedVector> = list()
            .iter()
            .filter(|v| v.questions().any(|q| q == question))
            .collect();
        for vector in &reaching {
            assert!(
                prefixes.iter().any(|p| vector.id.starts_with(p)),
                "{} reaches {} outside {prefixes:?}",
                vector.id,
                question.id()
            );
        }
        for prefix in prefixes {
            assert!(
                reaching.iter().any(|v| v.id.starts_with(prefix)),
                "{} names {prefix} but no such vector reaches it",
                question.id()
            );
        }
        reached += reaching.len();
    }
    assert!(reached > 1000, "{reached}");
}

fn word_value(bits: Known<u32>) -> u32 {
    match bits {
        Known::Value(v) => v,
        Known::Open(q) => panic!("open: {q:?}"),
    }
}

fn single(word: u32) -> Dyadic {
    super::model::extended_operand(word).0
}

#[test]
fn single_precision_follows_the_pages() {
    // [SPU-ISA p:196 s:9.1] truncation: 1 + 2^-24 keeps 1.0.
    let one_plus = Dyadic::new(false, (1 << 24) + 1, -24);
    let out = extended_result(&one_plus, false);
    assert_eq!(word_value(out.bits), 0x3F80_0000);
    assert_eq!(
        [out.overflow, out.underflow, out.diff],
        [Known::Value(false); 3]
    );
    // 1.5 x 1.5 = 2.25.
    let product = single(0x3FC0_0000).mul(&single(0x3FC0_0000));
    assert_eq!(
        word_value(extended_result(&product, false).bits),
        0x4010_0000
    );
    // At 2^129 the result saturates with OVF and DIFF.
    let out = extended_result(&Dyadic::new(true, 1, 129), false);
    assert_eq!(word_value(out.bits), 0xFFFF_FFFF);
    assert_eq!([out.overflow, out.diff], [Known::Value(true); 2]);
    // Just above Smax, OVF is the open band.
    let out = extended_result(&Dyadic::new(false, (1 << 25) - 1, 104), false);
    assert_eq!(word_value(out.bits), 0x7FFF_FFFF);
    assert_eq!(out.overflow, Known::Open(Question::SingleOverflowBand));
    // Below Smin: +0 with UNF and DIFF, whatever the sign.
    let out = extended_result(&Dyadic::new(true, 1, -127), false);
    assert_eq!(word_value(out.bits), 0);
    assert_eq!([out.underflow, out.diff], [Known::Value(true); 2]);
    // x + (-x) is +0.
    let sum = single(0xC040_0000).add(&single(0x4040_0000));
    assert_eq!(word_value(extended_result(&sum, false).bits), 0);
}

#[test]
fn fi_follows_the_formula() {
    // RB: exponent 127, base 0x1000 (1.5), step 0x200 (2^-4); RA: Y = 0.5.
    let rb = 127 << 23 | 0x1000 << 10 | 0x200;
    let ra = 127 << 23 | 0x4_0000;
    // 1.5 - 2^-4 x 0.5 = 1.46875.
    assert_eq!(word_value(interpolate(ra, rb).bits), 0x3FBC_0000);
    assert_eq!(
        interpolate(ra, 0x0000_1234).bits,
        Known::Open(Question::FiOperandFields)
    );
}

#[test]
fn conversions_saturate_at_the_integer_range() {
    // 2^31 saturates cflts; -1 saturates cfltu to 0.
    assert_eq!(to_integer(0x4F00_0000, 0, true), 0x7FFF_FFFF);
    assert_eq!(to_integer(0xCF00_0001, 0, true), 0x8000_0000);
    assert_eq!(to_integer(0xBF80_0000, 0, false), 0);
    assert_eq!(to_integer(0x3FC0_0000, 1, true), 3);
    // csflt of 1 at scale 127 is below Smin.
    let regs = BTreeMap::from([(4, 1u128 << 96)]);
    let step = effect(Insn::scaled(Op::Csflt, 3, 4, 155 - 127), &regs, 0);
    assert_eq!(step.writes[0].value, Known::Value(0));
    let unf = step.flags.iter().find(|f| f.name == "s0.unf").unwrap();
    assert_eq!(unf.raised, Known::Value(true));
}

#[test]
fn double_precision_rounds_in_each_mode() {
    let one_and = |ulps: u128, shift: i64| Dyadic::new(false, (1 << 53) + ulps, -53 - shift);
    // 1 + 2^-53 is half an ulp above 1.
    let half = one_and(1, 0);
    let bits = |mode| round(&half, DOUBLE, mode).bits;
    assert_eq!(bits(Mode::Nearest), 0x3FF0_0000_0000_0000);
    assert_eq!(bits(Mode::Up), 0x3FF0_0000_0000_0001);
    assert_eq!(bits(Mode::Down), 0x3FF0_0000_0000_0000);
    assert_eq!(bits(Mode::Zero), 0x3FF0_0000_0000_0000);
    assert!(round(&half, DOUBLE, Mode::Nearest).inexact);
    // A tie rounds to even: 1 + 3 x 2^-53 goes up to 1 + 2^-51.
    assert_eq!(
        round(&one_and(3, 0), DOUBLE, Mode::Nearest).bits,
        0x3FF0_0000_0000_0002
    );
    // Overflow: to infinity in nearest, to the largest finite toward zero.
    let huge = Dyadic::new(false, 1, 1024);
    assert_eq!(
        round(&huge, DOUBLE, Mode::Nearest).bits,
        0x7FF0_0000_0000_0000
    );
    assert_eq!(round(&huge, DOUBLE, Mode::Zero).bits, 0x7FEF_FFFF_FFFF_FFFF);
    assert!(round(&huge, DOUBLE, Mode::Zero).overflow);
    // The smallest denormal is tiny but exact: no UNF. Half of it is UNF.
    let dmin = round(&Dyadic::new(false, 1, -1074), DOUBLE, Mode::Nearest);
    assert_eq!((dmin.bits, dmin.underflow), (1, false));
    let half_dmin = round(&Dyadic::new(false, 3, -1076), DOUBLE, Mode::Nearest);
    assert_eq!(
        (half_dmin.bits, half_dmin.underflow, half_dmin.inexact),
        (1, true, true)
    );
    // frds: 1 + 2^-24 is half way between two singles.
    let x = 0x3FF0_0000_1000_0000;
    let left = |mode| match round_to_single(x, mode).pieces[0].2 {
        Known::Value(v) => v >> 32,
        Known::Open(q) => panic!("{q:?}"),
    };
    assert_eq!(left(Mode::Nearest), 0x3F80_0000);
    assert_eq!(left(Mode::Up), 0x3F80_0001);
    assert_eq!(
        round(&Dyadic::new(false, 3, -2), SINGLE, Mode::Nearest).bits,
        0x3F40_0000
    );
}

#[test]
fn double_special_values_follow_the_pages() {
    let inf = 0x7FF0_0000_0000_0000;
    // Infinity minus infinity is the default QNaN with INV.
    let out = double_slot(Op::Dfs, [inf, inf, 0], Mode::Nearest);
    assert_eq!(out.pieces[0].2, Known::Value(0x7FF8_0000_0000_0000));
    // x - x is +0, or -0 rounding toward -infinity.
    let one = 0x3FF0_0000_0000_0000;
    let zero = |mode| double_slot(Op::Dfs, [one, one, 0], mode).pieces[0].2;
    assert_eq!(zero(Mode::Nearest), Known::Value(0));
    assert_eq!(zero(Mode::Down), Known::Value(1 << 63));
    // dfnms of a NaN keeps sign 0; its payload is open.
    let out = double_slot(Op::Dfnms, [0xFFF8_0000_0000_0001, one, one], Mode::Nearest);
    assert_eq!(out.pieces[0].1, 0xFFF8_0000_0000_0000);
    assert_eq!(out.pieces[1].2, Known::Open(Question::DoubleNanResult));
    // dfnma negates the rounded sum: -(1 x 1 + 1) = -2.
    let out = double_slot(Op::Dfnma, [one, one, one], Mode::Nearest);
    assert_eq!(out.pieces[0].2, Known::Value(0xC000_0000_0000_0000));
}

/// [SPU-ISA p:199 s:9.2.2] an implementation that forces a denormal input to zero may set INV as well as DENORM, so INV stays open beside a denormal input.
#[test]
fn inv_is_open_beside_a_denormal_input() {
    let inv = |op: Op, ra: u128, rb: u128| {
        let regs = BTreeMap::from([(4, ra), (5, rb)]);
        effect(Insn::new(op, 3, 4, 5), &regs, 0)
            .flags
            .into_iter()
            .find(|f| f.name == "d0.inv")
            .unwrap()
            .raised
    };
    // A QNaN beside a denormal, a denormal frds input, a denormal fesd input.
    let qnan = 0x7FF8_0000_0000_0000u128 << 64;
    assert_eq!(
        inv(Op::Dfa, qnan, 1 << 64),
        Known::Open(Question::DoubleDenormalOperand)
    );
    assert_eq!(
        inv(Op::Frds, 1 << 64, 0),
        Known::Open(Question::ConversionDenormalInput)
    );
    assert_eq!(
        inv(Op::Fesd, 1 << 96, 0),
        Known::Open(Question::ConversionDenormalInput)
    );
}
