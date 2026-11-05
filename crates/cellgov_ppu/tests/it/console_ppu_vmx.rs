//! CellGov's vector loads, stores, permutes, packs, data streams and VSCR
//! moves against their console captures, and the PPE's left and right
//! vector load and store opcodes.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::instruction::{PpuDecodeError, PpuInstruction as I};

/// The primary opcode and 10-bit extended opcode of an X-form word.
fn primary_and_xo(word: u32) -> (u32, u32) {
    (word >> 26, (word >> 1) & 0x3FF)
}

const PROBES: [(&str, usize); 7] = [
    ("ppu_vmx_loads", 180),
    ("ppu_vmx_stores", 144),
    ("ppu_vmx_permute", 148),
    ("ppu_vmx_pack", 62),
    ("ppu_vmx_dst", 23),
    ("ppu_vscr_moves", 9),
    ("ppu_vscr_rules", 10),
];

#[test]
fn every_vector_memory_permute_pack_stream_and_vscr_case_matches_the_console_on_both_runs() {
    for (probe, count) in PROBES {
        let cases = table(probe);
        let Capture::Done(runs) = capture(probe, cases.len()) else {
            panic!("{probe}: the run ended inside a case");
        };
        assert_eq!(runs.len(), count, "{probe}");
        let mut wrong = Vec::new();
        for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
            let v = varies(case, runs);
            if !v.is_empty() {
                wrong.push(format!("{}: the runs differ in {v:?}", case.id));
            }
            match replay(probe, case, index, &runs[0]) {
                CaseOutcome::Pass => {}
                other => wrong.push(format!("{} [{}]: {other:?}", case.id, case.asm)),
            }
        }
        assert!(
            wrong.is_empty(),
            "{probe}: {} case(s):\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }
}

/// Whether `insn` is the left or right load or store the console runs for `xo`.
///
/// Each `ppu_may_trap_vxo<XO>` capture loads into v3 or stores into the
/// buffer, from the left or the right of EA.
fn is_console_form(xo: u32, insn: &I) -> bool {
    matches!(
        (xo, insn),
        (519, I::Lvlx { .. })
            | (551, I::Lvrx { .. })
            | (647, I::Stvlx { .. })
            | (679, I::Stvrx { .. })
            | (775, I::Lvlxl { .. })
            | (807, I::Lvrxl { .. })
            | (903, I::Stvlxl { .. })
            | (935, I::Stvrxl { .. })
    )
}

#[test]
fn each_left_and_right_vector_opcode_decodes_and_runs_as_on_the_console() {
    for xo in [519, 551, 647, 679, 775, 807, 903, 935] {
        let probe = format!("ppu_may_trap_vxo{xo}");
        let cases = table(&probe);
        let Capture::Done(runs) = capture(&probe, 1) else {
            panic!("{probe}: the console ended the run");
        };
        assert_eq!(primary_and_xo(cases[0].words[0]), (31, xo), "{probe}");
        let v = varies(&cases[0], &runs[0]);
        assert!(v.is_empty(), "{probe}: the runs differ in {v:?}");
        let insn = decode(cases[0].words[0]).unwrap_or_else(|e| panic!("{probe}: {e}"));
        assert!(is_console_form(xo, &insn), "{probe}: {insn:?}");
        assert_eq!(
            replay(&probe, &cases[0], 0, &runs[0][0]),
            CaseOutcome::Pass,
            "{probe}"
        );
    }
}

#[test]
fn the_four_other_left_and_right_opcodes_end_the_process_and_do_not_decode() {
    for xo in [583, 711, 839, 967] {
        let probe = format!("ppu_may_trap_vxo{xo}");
        let word = table(&probe)[0].words[0];
        assert_eq!(primary_and_xo(word), (31, xo), "{probe}");
        assert!(
            matches!(capture(&probe, 1), Capture::Ended { case: 0 }),
            "{probe}: the console ran it"
        );
        let decoded = decode(word);
        assert_eq!(
            decoded,
            Err(PpuDecodeError::EncodingNotRecognized { raw: word }),
            "{probe}"
        );
    }
}
