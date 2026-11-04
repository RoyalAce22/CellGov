//! CellGov's integer instructions against the `ppu_int_edges` console
//! capture, case by case through the differential harness.
//!
//! Two classes of row leave the bit-for-bit comparison:
//!
//! - `addc` and its forms, which CellGov does not decode yet. Their
//!   console answers wait in the capture.
//! - `divw` and `divwo`: [PPC-Book1 p:58 s:3.3.8] RT bits 0:31 are
//!   undefined, and so are the LT, GT and EQ bits of CR0 in 64-bit mode.
//!   The console leaves bits 0:31 zero and sets CR0 from that 64-bit
//!   value; CellGov sign-extends the quotient. The defined parts are
//!   compared.

use crate::console_cases::{capture, cellgov_after, replay, table, varies, Capture, Case, Run};
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

fn is_addc(case: &Case) -> bool {
    matches!(mnemonic(case), "addc" | "addco")
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
    let mut wrong = Vec::new();
    for (index, case, runs) in rows() {
        if is_addc(&case) || is_divw(&case) {
            continue;
        }
        compared += 1;
        match replay(PROBE, &case, index, &runs[0]) {
            CaseOutcome::Pass => {}
            other => wrong.push(format!("{} [{}]: {other:?}", case.id, case.asm)),
        }
    }
    assert_eq!(compared, 1917 - 98 - 41);
    assert!(
        wrong.is_empty(),
        "{} case(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn cellgov_does_not_decode_the_addc_rows_yet() {
    let mut rows_seen = 0;
    for (index, case, runs) in rows() {
        if !is_addc(&case) {
            continue;
        }
        rows_seen += 1;
        assert!(
            matches!(
                replay(PROBE, &case, index, &runs[0]),
                CaseOutcome::DecodeError(_)
            ),
            "{} decodes now: compare it with the console",
            case.id
        );
    }
    assert_eq!(rows_seen, 98);
}

#[test]
fn divw_matches_the_console_in_every_defined_part() {
    let mut rows_seen = 0;
    for (index, case, runs) in rows() {
        if !is_divw(&case) {
            continue;
        }
        rows_seen += 1;
        let console = &runs[0];
        let s = cellgov_after(&case, index).unwrap_or_else(|e| panic!("{}: {e:?}", case.id));
        assert_eq!(
            s.gpr[5] as u32, console.slots[0] as u32,
            "{} quotient",
            case.id
        );
        assert_eq!(s.xer, console.xer, "{} XER", case.id);
        assert_eq!(
            s.cr & !CR0_LT_GT_EQ,
            console.cr & !CR0_LT_GT_EQ,
            "{} CR outside CR0's LT, GT and EQ",
            case.id
        );
    }
    assert_eq!(rows_seen, 41);
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
