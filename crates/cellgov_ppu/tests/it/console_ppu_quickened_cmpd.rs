//! The quickened `cmpdi crF, rA, 0` against the `cmpdi` cases with a zero
//! immediate in the `ppu_fixed_point_logic` console capture.

use crate::console_cases::{capture, table, Capture};
use cellgov_event::UnitId;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_testkit::console_cases::Reg;

const PROBE: &str = "ppu_fixed_point_logic";

/// The ids of the probe's `cmpdi` cases with a zero immediate.
const ZERO_IMM_PREFIX: &str = "cmpi_l1_i0_";

/// Guest address of the shadowed word.
const BASE: u64 = 0x1_0000;

#[test]
fn quickened_cmpdi_zero_matches_the_console_for_every_zero_immediate_case() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    let mut checked = 0;
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        if !case.id.starts_with(ZERO_IMM_PREFIX) {
            continue;
        }
        let [word] = case.words[..] else {
            panic!("{}: one word", case.id);
        };
        let shadow = PredecodedShadow::build_quickened(BASE, &word.to_be_bytes());
        let insn = shadow.get(BASE).expect("the word decodes");
        assert!(
            matches!(insn, PpuInstruction::CmpdZero { .. }),
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
            assert_eq!(s.cr(), run.cr, "{} [{}]: cr", case.id, case.asm);
            assert_eq!(s.xer(), run.xer, "{} [{}]: xer", case.id, case.asm);
        }
        checked += 1;
    }
    assert_eq!(
        checked, 9,
        "the probe's nine cmpdi cases with a zero immediate"
    );
}
