//! The quickened `lis` against the `addis rT,0,imm` cases of the
//! `ppu_fixed_point_arith` console capture.

use crate::console_cases::{capture, table, Capture};
use cellgov_event::UnitId;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_testkit::console_cases::Reg;

const PROBE: &str = "ppu_fixed_point_arith";

/// The ids of the probe's `addis` cases with RA = 0.
const RA0_PREFIX: &str = "addis_ra0_";

/// Guest address of the shadowed word.
const BASE: u64 = 0x1_0000;

#[test]
fn quickened_lis_matches_the_console_for_every_addis_with_ra_0() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    let mut checked = 0;
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        if !case.id.starts_with(RA0_PREFIX) {
            continue;
        }
        let [word] = case.words[..] else {
            panic!("{}: one word", case.id);
        };
        let shadow = PredecodedShadow::build_quickened(BASE, &word.to_be_bytes());
        let insn = shadow.get(BASE).expect("the word decodes");
        assert!(
            matches!(insn, PpuInstruction::Lis { .. }),
            "{}: the shadow holds {insn:?}",
            case.id
        );
        let mut s = PpuState::new();
        case.initial(index, 0).apply(&mut s);
        let verdict = execute(
            &insn,
            &mut s,
            UnitId::new(0),
            &[],
            &mut Vec::new(),
            &mut StoreBuffer::new(),
        );
        assert!(
            matches!(verdict, ExecuteVerdict::Continue),
            "{}: {verdict:?}",
            case.id
        );
        for (reg, slot) in case.slots() {
            let Reg::Gpr(n) = reg else {
                panic!("{}: records {}", case.id, reg.name());
            };
            for run in runs {
                assert_eq!(
                    s.gpr[usize::from(n)],
                    run.slots[slot],
                    "{} [{}]: {}",
                    case.id,
                    case.asm,
                    reg.name()
                );
            }
        }
        for run in runs {
            assert_eq!(s.cr(), run.cr, "{}: cr", case.id);
            assert_eq!(s.xer(), run.xer, "{}: xer", case.id);
        }
        checked += 1;
    }
    assert_eq!(checked, 5, "the probe's five addis cases with RA = 0");
}
