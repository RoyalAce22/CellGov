//! The census rules for the FPRs and the VRs against the console captures
//! of the table-driven floating-point and vector probes.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_testkit::console_cases::{capture, table, Capture, Case, Reg, Run};

use super::cells::{cell_writes, Cells};
use super::constants::spr_move;
use super::fp_vector::{address_into_vector, cr_writes, VECTOR_SPAN};
use crate::decode::decode;
use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction;

/// The table-driven probes that run the floating-point and vector words
/// that set a CR field.
const CR_PROBES: [&str; 10] = [
    "ppu_fp_arith",
    "ppu_fp_fma",
    "ppu_fp_unary",
    "ppu_float_edges",
    "ppu_fpscr_ops",
    "ppu_mcrfs_ox",
    "ppu_vcmp_int_record",
    "ppu_vmx_float_cr6",
    "ppu_vfp_compare",
    "ppu_vmx_int_edges",
];

/// The probe of each vector load at each EA from 0 to 15 past an aligned
/// buffer.
const LOADS: &str = "ppu_vmx_loads";

/// The probe of each vector store at each EA from 0 to 15 past an aligned
/// buffer.
const STORES: &str = "ppu_vmx_stores";

/// Each case of `probe`, with its index and its two console runs.
fn ran(probe: &str) -> Vec<(usize, Case, [Run; 2])> {
    let cases = table(probe);
    let Capture::Done(runs) = capture(probe, cases.len()) else {
        panic!("{probe}: the console ran each case");
    };
    cases
        .into_iter()
        .zip(runs)
        .enumerate()
        .map(|(index, (case, runs))| (index, case, runs))
        .collect()
}

fn decoded(word: u32) -> PpuInstruction {
    decode(word).unwrap_or_else(|e| panic!("{word:#010x}: {e:?}"))
}

/// The cells that the census writes for any word of `words`: the CR
/// fields of [`cr_writes`] and the cells of [`cell_writes`].
fn written(words: &[u32]) -> Cells {
    words.iter().fold(Cells::NONE, |cells, &word| {
        let d = decoded(word);
        cell_writes(&d, spr_move(&d))
            .iter()
            .fold(cells.union(cr_writes(&d)), |cells, write| {
                cells.union(write.cells)
            })
    })
}

/// CR field `field` of `cr`. Field 0 is the most significant nibble.
fn nibble(cr: u32, field: u8) -> u32 {
    (cr >> (28 - 4 * u32::from(field))) & 0xF
}

/// The CR fields whose bits differ between two CR values.
fn changed(before: u32, after: u32) -> Cells {
    (0..8)
        .filter(|&field| nibble(before, field) != nibble(after, field))
        .fold(Cells::NONE, |cells, field| {
            cells.union(Cells::cr_field(field))
        })
}

/// The kinds of word that set a CR field from the FPRs, the VRs or FPSCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Setter {
    /// `fcmpu` or `fcmpo`.
    Compare,
    /// `mcrfs`.
    Mcrfs,
    /// A floating-point word with Rc = 1.
    FloatRecord,
    /// A vector compare with Rc = 1.
    VectorRecord,
}

fn setter(d: &PpuInstruction) -> Option<Setter> {
    use PpuInstruction as I;
    match *d {
        I::Fp63 {
            op: Fp63Op::Fcmpu | Fp63Op::Fcmpo,
            ..
        } => Some(Setter::Compare),
        I::Fp63 {
            op: Fp63Op::Mcrfs, ..
        } => Some(Setter::Mcrfs),
        I::Fp63 { rc: true, .. } | I::Fp59 { rc: true, .. } => Some(Setter::FloatRecord),
        I::Vx { op, rc: true, .. } if op.is_vxr_compare() => Some(Setter::VectorRecord),
        _ => None,
    }
}

/// [PPC-Book1 p:119 s:4.6.7] `fcmpu` and `fcmpo` place the result of the compare into CR field BF.
/// [PPC-Book1 p:120 s:4.6.8] `mcrfs` copies FPSCR field BFA into CR field BF.
/// [PPC-Book1 p:18 s:2.3.1] A floating-point instruction can set CR field 1 as an implicit result.
/// [AltiVec-PEM p:2-7 s:2.4] A vector compare with Rc = 1 updates CR6.
#[test]
fn each_cr_field_that_a_floating_point_or_vector_word_changed_on_the_console_is_one_the_census_writes(
) {
    let mut seen = BTreeMap::<Setter, usize>::new();
    for probe in CR_PROBES {
        for (_, case, runs) in ran(probe) {
            for run in &runs {
                let changed = changed(case.cr, run.cr);
                let outside = changed.without(written(&case.words));
                assert!(
                    outside.is_empty(),
                    "{probe} {} ({}): the console changed {outside:?}, a CR field that the census does not write",
                    case.id,
                    case.asm
                );
                if changed.is_empty() {
                    continue;
                }
                for &word in &case.words {
                    if let Some(kind) = setter(&decoded(word)) {
                        *seen.entry(kind).or_default() += 1;
                    }
                }
            }
        }
    }
    for kind in [
        Setter::Compare,
        Setter::Mcrfs,
        Setter::FloatRecord,
        Setter::VectorRecord,
    ] {
        assert!(
            seen.get(&kind).copied().unwrap_or_default() > 0,
            "{kind:?}: no run changed a CR field"
        );
    }
}

/// The bytes of the VR that `case` records after `run`, each `None`
/// where the VR still holds the byte that the harness loaded before the
/// case.
fn vr_written(case: &Case, index: usize, run: &Run) -> Vec<Option<u8>> {
    let (reg, slot) = case
        .slots()
        .into_iter()
        .find(|&(reg, _)| matches!(reg, Reg::Vr(_)))
        .unwrap_or_else(|| panic!("{}: no VR record", case.id));
    let before = case
        .loads(index)
        .iter()
        .find(|&&(r, _)| r == reg)
        .map(|&(_, value)| value)
        .unwrap_or_else(|| panic!("{}: no load of the VR", case.id));
    let after = u128::from(run.slots[slot]) << 64 | u128::from(run.slots[slot + 1]);
    before
        .to_be_bytes()
        .into_iter()
        .zip(after.to_be_bytes())
        .map(|(b, a)| (a != b).then_some(a))
        .collect()
}

/// The cases of one word over one memory image differ only in EA. So the
/// VR moves with EA when two of their runs write different bytes.
///
/// The census also names the element loads, whose VR keeps still on the
/// console. That name costs only precision.
///
/// [AltiVec-PEM p:6-21 s:6.2] `lvx` loads the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [AltiVec-PEM p:6-18 s:6.2] `lvsl` places a vector computed from sh = EA\[28:31\] into VD.
/// [AltiVec-PEM p:6-14 s:6.2] `lvebx` loads the byte at EA into byte EA\[28:31\] of VD.
#[test]
fn a_vector_load_whose_vr_moves_with_ea_on_the_console_is_one_the_census_takes_bits_of_ea_from() {
    type Group = (usize, BTreeSet<Vec<Option<u8>>>);
    let mut groups = BTreeMap::<(Vec<u32>, Option<Vec<u8>>), Group>::new();
    for (index, case, runs) in ran(LOADS) {
        let group = groups
            .entry((case.words.clone(), case.mem.clone()))
            .or_default();
        group.0 += 1;
        for run in &runs {
            group.1.insert(vr_written(&case, index, run));
        }
    }
    let (mut moved, mut unnamed) = (0, 0);
    for ((words, _), (cases, results)) in &groups {
        if *cases < 2 {
            continue;
        }
        let [word] = words[..] else {
            panic!("{words:x?}: a case of more than one word");
        };
        let d = decoded(word);
        let named = address_into_vector(&d) != 0;
        if results.len() > 1 {
            assert!(
                named,
                "{d:?}: the console VR moves with EA, and the census takes no bit of EA"
            );
            moved += 1;
        } else {
            unnamed += usize::from(!named);
        }
    }
    assert!(moved > 0, "no load whose VR moves with EA");
    assert!(
        unnamed > 0,
        "no load whose VR keeps still that the census leaves unnamed"
    );
}

/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [CBE-Handbook p:710 s:24.9] `stvlx` and `stvrx` store the left and right parts of a quadword at an unaligned EA.
#[test]
fn each_byte_that_a_vector_store_wrote_on_the_console_lies_in_the_census_span_of_its_ea() {
    let (first, width) = VECTOR_SPAN;
    let (mut written, mut below) = (0, 0);
    for (index, case, runs) in ran(STORES) {
        assert!(
            case.asm.ends_with(", r10, r4"),
            "{}: EA = r10 + r4",
            case.id
        );
        let ea = case
            .loads(index)
            .iter()
            .find(|&&(reg, _)| reg == Reg::Gpr(4))
            .map(|&(_, value)| i64::try_from(value).expect("an offset in the buffer"))
            .unwrap_or_else(|| panic!("{}: no load of r4", case.id));
        let mem = case
            .mem
            .as_ref()
            .unwrap_or_else(|| panic!("{}: no buffer", case.id));
        for run in &runs {
            for (reg, slot) in case.slots() {
                let Reg::Mem(off) = reg else { continue };
                for (k, byte) in run.slots[slot].to_be_bytes().into_iter().enumerate() {
                    let at = usize::from(off) + k;
                    if byte == mem[at] {
                        continue;
                    }
                    let at = i64::try_from(at).expect("an offset in the buffer");
                    assert!(
                        (ea + first..ea + first + i64::from(width)).contains(&at),
                        "{} ({}): the console wrote byte {at}, outside the span of EA {ea}",
                        case.id,
                        case.asm
                    );
                    written += 1;
                    below += usize::from(at < ea);
                }
            }
        }
    }
    assert!(written > 0, "no store wrote a byte");
    assert!(below > 0, "no store wrote a byte below its EA");
}
