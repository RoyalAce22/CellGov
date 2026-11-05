//! `sync` with L = 3 against its may-trap console capture.
//!
//! [PPC-Book2 p:26 s:3.3.3] L = 3 is reserved. The console runs it and
//! the process continues. CellGov decodes it as a `sync` that orders
//! nothing more than `sync` L = 0.

use crate::console_cases::{capture, replay, table, Capture};
use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::instruction::PpuInstruction;

#[test]
fn sync_l3_runs_on_the_console_and_cellgov_matches_each_run() {
    let name = "ppu_may_trap_sync_l3";
    let cases = table(name);
    assert_eq!(cases.len(), 1, "{name}: one case");
    assert!(cases[0].may_end_process, "{name}: a may-trap case");
    let Capture::Done(runs) = capture(name, 1) else {
        panic!("{name}: the console ended the run");
    };
    assert_eq!(decode(cases[0].words[0]), Ok(PpuInstruction::Sync { l: 3 }));
    for (index, run) in runs[0].iter().enumerate() {
        assert_eq!(
            replay(name, &cases[0], 0, run),
            CaseOutcome::Pass,
            "{name}: run {index}"
        );
    }
}
