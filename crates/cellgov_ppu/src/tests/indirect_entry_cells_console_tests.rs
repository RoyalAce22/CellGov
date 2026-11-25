//! The census cells of CR and XER against the console captures of the
//! table-driven fixed-point, CR and XER probes.

use cellgov_ps3_abi::hw::ppu_status::{XER_CA, XER_OV, XER_SO};
use cellgov_testkit::console_cases::{capture, table, Capture, Case, Reg, Run};

use super::cells::{cell_writes, computed_cells, read_cells, CellWrite, Cells};
use super::constants::{spr_move, SprMove};
use crate::decode::decode;
use crate::instruction::PpuInstruction;

/// The table-driven probes whose words write CR and XER only as
/// fixed-point, CR and XER words.
const PROBES: [&str; 11] = [
    "ppu_int_edges",
    "ppu_cr_moves",
    "ppu_spr_moves",
    "ppu_fixed_point_arith",
    "ppu_fixed_point_logic",
    "ppu_fixed_point_rotate",
    "ppu_addc_xer",
    "ppu_ocrf_fxm",
    "ppu_divw_quotient",
    "ppu_mulhwu_high_bit",
    "ppu_reference_vectors",
];

/// The word that the probe harness loads CR with before each run:
/// `mtcrf 0xff, r0`.
const HARNESS_MTCRF: u32 = 0x7c0f_f120;

/// The word that the probe harness loads XER with before each run:
/// `mtxer r0`.
const HARNESS_MTXER: u32 = 0x7c01_03a6;

/// One console run of one case.
struct Ran {
    probe: &'static str,
    case: Case,
    index: usize,
    run: Run,
}

/// Each run of each case of [`PROBES`], in the order that the console ran
/// them.
fn runs() -> Vec<Ran> {
    let mut out = Vec::new();
    for probe in PROBES {
        let cases = table(probe);
        let Capture::Done(runs) = capture(probe, cases.len()) else {
            panic!("{probe}: the console ran each case");
        };
        for (index, (case, pair)) in cases.into_iter().zip(runs).enumerate() {
            for run in pair {
                out.push(Ran {
                    probe,
                    case: case.clone(),
                    index,
                    run,
                });
            }
        }
    }
    out
}

fn decoded(word: u32) -> PpuInstruction {
    decode(word).unwrap_or_else(|e| panic!("{word:#010x}: {e:?}"))
}

/// The census writes of `word` into the cells.
fn writes(word: u32) -> [CellWrite; 3] {
    let d = decoded(word);
    cell_writes(&d, spr_move(&d))
}

/// The cells that the census writes for any word of `words`.
fn written(words: &[u32]) -> Cells {
    words
        .iter()
        .flat_map(|&word| writes(word))
        .fold(Cells::NONE, |cells, write| cells.union(write.cells))
}

/// CR field `field` of `cr`. Field 0 is the most significant nibble.
fn nibble(cr: u32, field: u8) -> u32 {
    (cr >> (28 - 4 * u32::from(field))) & 0xF
}

/// The cells whose bits differ between two CR and XER values.
fn changed(cr: (u32, u32), xer: (u64, u64)) -> Cells {
    let mut cells = Cells::NONE;
    for field in 0..8 {
        if nibble(cr.0, field) != nibble(cr.1, field) {
            cells = cells.union(Cells::cr_field(field));
        }
    }
    let diff = xer.0 ^ xer.1;
    if diff & XER_CA != 0 {
        cells = cells.union(Cells::CA);
    }
    if diff & (XER_SO | XER_OV) != 0 {
        cells = cells.union(Cells::OV);
    }
    if diff & !(XER_SO | XER_OV | XER_CA) != 0 {
        cells = cells.union(Cells::SPR);
    }
    cells
}

/// The value that the harness loads into GPR `gpr` before the case.
fn gpr_in(ran: &Ran, gpr: u8) -> u64 {
    ran.case
        .loads(ran.index)
        .iter()
        .find(|&&(reg, _)| reg == Reg::Gpr(gpr))
        .map(|&(_, value)| value as u64)
        .unwrap_or_else(|| panic!("{} {}: no load of r{gpr}", ran.probe, ran.case.id))
}

/// The value that the console recorded for GPR `gpr` after the case.
fn gpr_out(ran: &Ran, gpr: u8) -> u64 {
    let (_, slot) = ran
        .case
        .slots()
        .into_iter()
        .find(|&(reg, _)| reg == Reg::Gpr(gpr))
        .unwrap_or_else(|| panic!("{} {}: no record of r{gpr}", ran.probe, ran.case.id));
    ran.run.slots[slot]
}

/// The last word of a case whose earlier words write no cell.
fn last_word_alone(case: &Case) -> Option<u32> {
    let (&last, earlier) = case.words.split_last()?;
    written(earlier).is_empty().then_some(last)
}

#[test]
fn each_cr_field_and_xer_bit_that_a_word_changed_on_the_console_is_a_cell_that_the_census_writes() {
    let runs = runs();
    let mut changes = 0;
    for ran in &runs {
        let changed = changed((ran.case.cr, ran.run.cr), (ran.case.xer, ran.run.xer));
        let outside = changed.without(written(&ran.case.words));
        assert!(
            outside.is_empty(),
            "{} {} ({}): the console changed {outside:?}, a cell that the census does not write",
            ran.probe,
            ran.case.id,
            ran.case.asm
        );
        changes += usize::from(!changed.is_empty());
    }
    assert!(changes > 0, "no run changed CR or XER");
}

/// The kinds of word whose full write of a CR field the console checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FullCr {
    /// A compare or a record form: LT, GT or EQ, and SO.
    Compare,
    /// `stwcx.` or `stdcx.`: 0b00, the store bit, and SO.
    StoreConditional,
    /// `mcrf`: CR field BFA.
    Mcrf,
    /// `mtcrf`, or `mtocrf` with one FXM bit: the field of RS.
    Move,
}

#[test]
fn a_full_census_write_of_a_cr_field_sets_each_bit_of_the_field_on_the_console() {
    let mut seen = std::collections::BTreeMap::<FullCr, (usize, usize)>::new();
    for ran in &runs() {
        let Some(word) = last_word_alone(&ran.case) else {
            continue;
        };
        let d = decoded(word);
        for write in writes(word) {
            if !write.full || write.cells.is_empty() || !write.cells.without(Cells::CR).is_empty() {
                continue;
            }
            for field in (0..8).filter(|&f| write.cells.meets(Cells::cr_field(f))) {
                let before = nibble(ran.case.cr, field);
                let after = nibble(ran.run.cr, field);
                let so = u32::from(ran.run.xer & XER_SO != 0);
                let at = format!("{} {} ({}) CR{field}", ran.probe, ran.case.id, ran.case.asm);
                let kind = match d {
                    PpuInstruction::Mtcrf { rs, .. } | PpuInstruction::Mtocrf { rs, .. } => {
                        let rs = gpr_in(ran, rs) as u32;
                        assert_eq!(after, nibble(rs, field), "{at}: the field of RS");
                        FullCr::Move
                    }
                    PpuInstruction::Mcrf { crfs, .. } => {
                        assert_eq!(after, nibble(ran.case.cr, crfs), "{at}: field BFA");
                        FullCr::Mcrf
                    }
                    PpuInstruction::Stwcx { .. } | PpuInstruction::Stdcx { .. } => {
                        assert_eq!(after & 0b1101, so, "{at}: 0b00 || n || SO");
                        FullCr::StoreConditional
                    }
                    _ => {
                        assert_eq!((after >> 1).count_ones(), 1, "{at}: one of LT, GT, EQ");
                        assert_eq!(after & 1, so, "{at}: SO");
                        FullCr::Compare
                    }
                };
                let entry = seen.entry(kind).or_default();
                entry.0 += 1;
                entry.1 += usize::from(before & !after != 0);
            }
        }
    }
    for kind in [
        FullCr::Compare,
        FullCr::StoreConditional,
        FullCr::Mcrf,
        FullCr::Move,
    ] {
        let (runs, cleared) = seen.get(&kind).copied().unwrap_or_default();
        assert!(runs > 0, "{kind:?}: no run");
        assert!(cleared > 0, "{kind:?}: no run cleared a bit that was set");
    }
}

#[test]
fn mtxer_and_mtcrf_write_each_bit_of_the_xer_and_cr_cells_that_the_harness_loads() {
    let [mtxer, ..] = writes(HARNESS_MTXER);
    assert!(mtxer.full, "mtxer: a full write");
    assert_eq!(
        mtxer.cells,
        Cells::CA.union(Cells::OV),
        "mtxer: CA, OV and SO"
    );
    let [mtcrf, ..] = writes(HARNESS_MTCRF);
    assert!(mtcrf.full, "mtcrf 0xff: a full write");
    assert_eq!(mtcrf.cells, Cells::CR, "mtcrf 0xff: each CR field");

    let mut cleared = (0, 0);
    let runs = runs();
    for pair in runs.windows(2) {
        let [before, ran] = pair else { continue };
        if before.probe != ran.probe {
            continue;
        }
        let words = written(&ran.case.words);
        let at = format!("{} {} ({})", ran.probe, ran.case.id, ran.case.asm);
        let flags = XER_SO | XER_OV | XER_CA;
        if !words.meets(Cells::CA.union(Cells::OV)) && before.run.xer & flags & !ran.case.xer != 0 {
            assert_eq!(ran.run.xer & flags, ran.case.xer & flags, "{at}: XER flags");
            cleared.0 += 1;
        }
        if !words.meets(Cells::CR) && before.run.cr & !ran.case.cr != 0 {
            assert_eq!(ran.run.cr, ran.case.cr, "{at}: CR");
            cleared.1 += 1;
        }
    }
    assert!(
        cleared.0 > 0,
        "no run cleared an XER flag that the run before left set"
    );
    assert!(
        cleared.1 > 0,
        "no run cleared a CR bit that the run before left set"
    );
}

#[test]
fn mfcr_mfocrf_mfxer_and_each_spr_move_out_read_the_cells_whose_bits_they_return() {
    let mut reads = (0, 0, 0);
    for ran in &runs() {
        let at = format!("{} {} ({})", ran.probe, ran.case.id, ran.case.asm);
        match ran.case.words[..] {
            [word] => match decoded(word) {
                d @ (PpuInstruction::Mfcr { rt } | PpuInstruction::Mfocrf { rt, .. }) => {
                    assert_eq!(gpr_out(ran, rt) as u32, ran.case.cr, "{at}: the whole CR");
                    assert_eq!(read_cells(&d), Cells::CR, "{at}: each CR field");
                    reads.0 += 1;
                }
                d @ PpuInstruction::Mfxer { rt } => {
                    let value = gpr_out(ran, rt);
                    assert_eq!(value, ran.case.xer, "{at}: XER");
                    let returned = changed((0, 0), (0, value));
                    assert!(
                        returned.without(read_cells(&d)).is_empty(),
                        "{at}: {returned:?}"
                    );
                    reads.1 += 1;
                }
                _ => {}
            },
            [into, out] => {
                let (din, dout) = (decoded(into), decoded(out));
                let (Some(_), Some(SprMove::OutOf(rt))) = (spr_move(&din), spr_move(&dout)) else {
                    continue;
                };
                if gpr_out(ran, rt) == 0 {
                    continue;
                }
                assert!(
                    written(&[into]).meets(read_cells(&dout)),
                    "{at}: the move out reads a cell that the move in wrote"
                );
                reads.2 += 1;
            }
            _ => {}
        }
    }
    assert!(reads.0 > 0, "no mfcr or mfocrf run");
    assert!(reads.1 > 0, "no mfxer run");
    assert!(reads.2 > 0, "no move-in, move-out run");
}

#[test]
fn each_word_that_adds_xer_ca_into_its_result_on_the_console_computes_from_the_ca_cell() {
    let mut carried = 0;
    for ran in &runs() {
        let [word] = ran.case.words[..] else {
            continue;
        };
        let d = decoded(word);
        let ca = u64::from(ran.case.xer & XER_CA != 0);
        let sum = |rt: u8, a: u64, b: u64| Some((rt, a.wrapping_add(b)));
        let input = |gpr| gpr_in(ran, gpr);
        let Some((rt, base)) = (match d {
            PpuInstruction::Adde { rt, ra, rb, .. } => sum(rt, input(ra), input(rb)),
            PpuInstruction::Subfe { rt, ra, rb, .. } => sum(rt, !input(ra), input(rb)),
            PpuInstruction::Addme { rt, ra, .. } => sum(rt, input(ra), u64::MAX),
            PpuInstruction::Subfme { rt, ra, .. } => sum(rt, !input(ra), u64::MAX),
            PpuInstruction::Addze { rt, ra, .. } => sum(rt, input(ra), 0),
            PpuInstruction::Subfze { rt, ra, .. } => sum(rt, !input(ra), 0),
            _ => None,
        }) else {
            continue;
        };
        let at = format!("{} {} ({})", ran.probe, ran.case.id, ran.case.asm);
        assert_eq!(
            gpr_out(ran, rt),
            base.wrapping_add(ca),
            "{at}: the sum with CA"
        );
        if ca == 1 {
            assert!(
                computed_cells(&d).meets(Cells::CA),
                "{at}: computes from CA"
            );
            carried += 1;
        }
    }
    assert!(carried > 0, "no run with CA set");
}
