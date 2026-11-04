//! CellGov's integer instructions against the `ppu_int_edges` console
//! capture, case by case through the differential harness.
//!
//! [PPC-Book1 p:58 s:3.3.8] `divw` leaves RT bits 0:31 undefined, and the LT, GT and EQ bits of CR0 in 64-bit mode.
//! The console leaves bits 0:31 zero and sets CR0 from that 64-bit
//! value, and CellGov does the same, so `divw` and `divwo` replay bit
//! for bit with the other rows.

use crate::console_cases::{capture, replay, table, varies, Capture, Case, Run};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_int_edges";

/// CR field 0's LT, GT and EQ bits.
const CR0_LT_GT_EQ: u32 = 0xE000_0000;

fn rows() -> Vec<(usize, Case, [Run; 2])> {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 1917);
    cases
        .into_iter()
        .zip(runs)
        .enumerate()
        .map(|(i, (c, r))| (i, c, r))
        .collect()
}

fn mnemonic(case: &Case) -> &str {
    case.asm.split([' ', '.']).next().unwrap_or_default()
}

fn is_divw(case: &Case) -> bool {
    matches!(mnemonic(case), "divw" | "divwo")
}

#[test]
fn the_two_console_runs_of_every_case_agree() {
    let varying: Vec<String> = rows()
        .iter()
        .filter_map(|(_, c, r)| {
            let v = varies(c, r);
            (!v.is_empty()).then(|| format!("{}: {v:?}", c.id))
        })
        .collect();
    assert!(varying.is_empty(), "{}", varying.join("\n"));
}

#[test]
fn every_integer_edge_case_matches_the_console() {
    let mut compared = 0;
    let mut addc = 0;
    let mut divw = 0;
    let mut wrong = Vec::new();
    for (index, case, runs) in rows() {
        compared += 1;
        addc += u32::from(matches!(mnemonic(&case), "addc" | "addco"));
        divw += u32::from(is_divw(&case));
        match replay(PROBE, &case, index, &runs[0]) {
            CaseOutcome::Pass => {}
            other => wrong.push(format!("{} [{}]: {other:?}", case.id, case.asm)),
        }
    }
    assert_eq!(compared, 1917);
    assert_eq!(addc, 98, "the addc and addco rows");
    assert_eq!(divw, 41, "the divw and divwo rows");
    assert!(
        wrong.is_empty(),
        "{} case(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_console_leaves_the_high_word_of_a_divw_quotient_zero() {
    let mut negative = 0;
    for (_, case, runs) in rows() {
        if !is_divw(&case) {
            continue;
        }
        let rt = runs[0].slots[0];
        assert_eq!(rt >> 32, 0, "{}: RT {rt:#018x}", case.id);
        let gt = 0x4000_0000;
        let cr0 = runs[0].cr & CR0_LT_GT_EQ;
        assert_eq!(
            cr0,
            if rt == 0 { 0x2000_0000 } else { gt },
            "{} CR0",
            case.id
        );
        negative += u32::from((rt as u32) >> 31 == 1);
    }
    assert!(negative > 0, "no row had a negative quotient");
}
