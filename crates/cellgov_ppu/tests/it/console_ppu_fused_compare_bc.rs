//! The fused `cmpdi; bc`, `cmpd; bc` and `cmplw; bc` against the
//! `cmpdi`, `cmpd` and `cmplw` cases in the `ppu_fixed_point_logic`
//! console capture.

use crate::console_cases::{capture, table, Capture};
use cellgov_event::UnitId;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::{FusedPair, PpuInstruction};
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_testkit::console_cases::Reg;

const PROBE: &str = "ppu_fixed_point_logic";

/// Guest address of the compare word; the `bc` follows it.
const BASE: u64 = 0x1_0000;

/// Displacement of the `bc`.
const OFFSET: u32 = 16;

/// [PPC-Book1 p:20 s:2.4.1] BO=011at branches when the selected CR bit is one.
const BO_IF_TRUE: u32 = 0b01100;

/// The fused form that `id`'s compare takes, or `None` for a case of
/// another instruction.
fn fused_kind(id: &str) -> Option<&'static str> {
    [
        ("cmpi_l1_", "CmpdiBc"),
        ("cmp_l1_", "CmpdBc"),
        ("cmpl_l0_", "CmplwBc"),
    ]
    .into_iter()
    .find(|(prefix, _)| id.starts_with(prefix))
    .map(|(_, kind)| kind)
}

fn kind_of(insn: &PpuInstruction) -> &'static str {
    match insn {
        PpuInstruction::Fused(FusedPair::CmpdiBc { .. }) => "CmpdiBc",
        PpuInstruction::Fused(FusedPair::CmpdBc { .. }) => "CmpdBc",
        PpuInstruction::Fused(FusedPair::CmplwBc { .. }) => "CmplwBc",
        _ => "unfused",
    }
}

/// `bc BO_IF_TRUE, bi, OFFSET`: OPCD 16, AA = 0, LK = 0.
fn bc_word(bi: u32) -> u32 {
    (16 << 26) | (BO_IF_TRUE << 21) | (bi << 16) | OFFSET
}

#[test]
fn fused_compare_and_branch_matches_the_console_for_every_compare_case() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    let mut checked = 0;
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        let Some(kind) = fused_kind(&case.id) else {
            continue;
        };
        let [word] = case.words[..] else {
            panic!("{}: one word", case.id);
        };
        let bf = (word >> 23) & 7;
        for bit in 0..4 {
            let bi = bf * 4 + bit;
            let mut bytes = word.to_be_bytes().to_vec();
            bytes.extend_from_slice(&bc_word(bi).to_be_bytes());
            let shadow = PredecodedShadow::build(BASE, &bytes);
            let insn = shadow.get(BASE).expect("the word decodes");
            assert_eq!(
                kind_of(&insn),
                kind,
                "{}: the shadow holds {insn:?}",
                case.id
            );

            let mut s = PpuState::new();
            case.initial(index, 0).apply(&mut s);
            s.pc = BASE;
            let verdict = execute(
                &insn,
                &mut s,
                UnitId::new(0),
                &[],
                &mut Vec::new(),
                &mut StoreBuffer::new(),
            );
            let taken = match verdict {
                ExecuteVerdict::Branch => true,
                ExecuteVerdict::Continue => false,
                other => panic!("{}: {other:?}", case.id),
            };
            for run in runs {
                assert_eq!(s.cr(), run.cr, "{} [{}] bi {bi}: cr", case.id, case.asm);
                assert_eq!(s.xer(), run.xer, "{} [{}] bi {bi}: xer", case.id, case.asm);
                let console_bit = (run.cr >> (31 - bi)) & 1 == 1;
                let expected_pc = if console_bit {
                    BASE + 4 + u64::from(OFFSET)
                } else {
                    BASE
                };
                assert_eq!(
                    (taken, s.pc),
                    (console_bit, expected_pc),
                    "{} [{}] bi {bi}: branch",
                    case.id,
                    case.asm
                );
                for (reg, slot) in case.slots() {
                    let Reg::Gpr(n) = reg else {
                        panic!("{}: records {}", case.id, reg.name());
                    };
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
        }
        checked += 1;
    }
    assert_eq!(checked, 77, "the probe's cmpdi, cmpd and cmplw cases");
}
