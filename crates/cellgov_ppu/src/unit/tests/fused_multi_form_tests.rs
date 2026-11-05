//! The multi-form pairs leave the unit, and its timing, where their two
//! instructions leave them unfused.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::encode::encode;
use crate::instruction::{FusedPair, PpuInstruction as I};

/// Where the data words start; r1 and r7 point here.
const DATA: u64 = 0x80;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodes")
}

fn addi_r3() -> u32 {
    word(I::Addi {
        rt: 3,
        ra: 3,
        imm: 1,
    })
}

fn or(ra: u8, rs: u8, rb: u8, rc: bool) -> u32 {
    word(I::Or { ra, rs, rb, rc })
}

/// What a run left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    retired: u64,
    cost: u64,
    state: cellgov_exec::PpuFingerprint,
}

/// Runs `code` at 0, fused or by plain decode, in batches of `budget`,
/// until the PC passes the end of `code` or eight batches ran. Returns
/// the last batch's stop, with the cost summed over every batch.
fn run(code: &[u32], fused: bool, budget: u64) -> Stop {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let data = (0..16u8).map(|i| i.wrapping_mul(0x37)).collect::<Vec<_>>();
    let mut mem = GuestMemory::new(0x100);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(DATA), data.len() as u64).unwrap(),
        &data,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    let state = unit.state_mut();
    state.set_gpr(0, 0x7000);
    state.set_gpr(1, DATA);
    state.set_gpr(3, 0x8123_4567_89AB_CDEF);
    state.set_gpr(4, 0x0FED_CBA9_8765_4321);
    state.set_gpr(5, 0);
    state.set_gpr(7, DATA);
    state.set_ctr(4 * code.len() as u64);
    let mut cost = 0;
    let mut last = None;
    for _ in 0..8 {
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        cost += r.consumed_cost.raw();
        last = Some((r.yield_reason, r.local_diagnostics.pc));
        if unit.state().pc >= 4 * code.len() as u64 {
            break;
        }
    }
    let (reason, pc) = last.expect("one batch ran");
    Stop {
        reason,
        pc,
        retired: unit.retired_instructions(),
        cost,
        state: unit.state().fingerprint(),
    }
}

/// Whether the shadow fuses the word at 4 in `code`.
fn fuses_at_4(code: &[u32]) -> bool {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    matches!(
        shadow::PredecodedShadow::build(0, &bytes).get(4),
        Some(I::Fused(_))
    )
}

/// Each pair, preceded by an addi and followed by one, runs fused as it
/// runs split, at every budget.
fn assert_runs_as_split(pairs: &[(u32, u32)]) {
    for &(first, second) in pairs {
        let code = [addi_r3(), first, second, addi_r3(), addi_r3()];
        assert!(fuses_at_4(&code), "{first:#010x} then {second:#010x} fuses");
        for budget in [1, 2, 3, 64] {
            assert_eq!(
                run(&code, true, budget),
                run(&code, false, budget),
                "{first:#010x} then {second:#010x} budget {budget}"
            );
        }
    }
}

#[test]
fn rldicr_then_oris_runs_and_times_as_the_two() {
    let rldicr = |sh, me, rc| {
        word(I::Rldicr {
            ra: 5,
            rs: 4,
            sh,
            me,
            rc,
        })
    };
    let oris = |rs| {
        word(I::Oris {
            ra: 6,
            rs,
            imm: 0x8001,
        })
    };
    assert_runs_as_split(&[
        (rldicr(8, 40, true), oris(5)),
        (rldicr(4, 59, false), oris(5)),
        (rldicr(0, 63, true), oris(4)),
    ]);
}

#[test]
fn or_then_or_runs_and_times_as_the_two() {
    assert_runs_as_split(&[
        (or(5, 4, 3, false), or(6, 5, 3, true)),
        (or(5, 4, 4, false), or(6, 5, 5, false)),
        (or(4, 4, 3, true), or(3, 3, 3, false)),
    ]);
}

/// The cases include:
/// - a bc on CR0 after a recording or
/// - a bc on a CR field that the or does not change
/// - a bc that decrements CTR
/// - a bc whose target is the pair's fall-through or the bc's own slot
#[test]
fn or_then_bc_runs_and_times_as_the_two() {
    let bc = |bo, bi, offset| {
        word(I::Bc {
            bo,
            bi,
            offset,
            aa: false,
            link: false,
        })
    };
    assert_runs_as_split(&[
        (or(5, 4, 3, true), bc(12, 2, 8)),
        (or(5, 5, 5, true), bc(4, 2, 8)),
        (or(5, 4, 4, false), bc(12, 30, 4)),
        (or(5, 4, 3, false), bc(16, 0, 0)),
        (or(5, 4, 3, true), bc(12, 0, -8)),
    ]);
}

/// The lwz base is one of:
/// - the or's RA
/// - another register
/// - 0
///
/// The fourth case reads past memory, so the pair refuses and the or
/// reruns split on the RA it reads and writes. The last case writes r0,
/// which the lwz base still reads as 0.
#[test]
fn or_then_lwz_runs_and_times_as_the_two() {
    let lwz = |rt, ra, imm| word(I::Lwz { rt, ra, imm });
    assert_runs_as_split(&[
        (or(5, 7, 5, false), lwz(6, 5, 4)),
        (or(5, 4, 3, true), lwz(6, 7, 8)),
        (or(5, 4, 3, false), lwz(5, 0, 0x84)),
        (or(7, 7, 4, true), lwz(6, 7, 0)),
        (or(0, 7, 7, false), lwz(6, 0, 0)),
    ]);
}

/// CTR holds one of:
/// - the end of the code
/// - the address of the or
/// - the address of the mtctr
#[test]
fn or_then_bctrl_runs_and_times_as_the_two() {
    let bctrl = word(I::Bcctr {
        bo: 20,
        bi: 0,
        link: true,
    });
    let mtctr = |rs| word(I::Mtctr { rs });
    let li = |rt, imm| word(I::Li { rt, imm });
    for target in [20i16, 8, 4] {
        let code = [li(9, target), mtctr(9), or(5, 4, 3, true), bctrl, addi_r3()];
        let bytes = code
            .iter()
            .flat_map(|w| w.to_be_bytes())
            .collect::<Vec<_>>();
        assert!(matches!(
            shadow::PredecodedShadow::build(0, &bytes).get(8),
            Some(I::Fused(FusedPair::OrBctrl { .. }))
        ));
        for budget in [1, 2, 3, 64] {
            assert_eq!(
                run(&code, true, budget),
                run(&code, false, budget),
                "target {target} budget {budget}"
            );
        }
    }
}

#[test]
fn ld_then_addi_runs_and_times_as_the_two() {
    let ld = |imm| word(I::Ld { rt: 5, ra: 1, imm });
    let addi = |ra| word(I::Addi { rt: 6, ra, imm: -4 });
    let addi_r5 = word(I::Addi {
        rt: 5,
        ra: 5,
        imm: -4,
    });
    assert_runs_as_split(&[
        (ld(0), addi(5)),
        (ld(8), addi(0)),
        (ld(0x100), addi(1)),
        (ld(0), addi_r5),
    ]);
}

#[test]
fn lwz_then_or_runs_and_times_as_the_two() {
    let lwz = |imm| word(I::Lwz { rt: 5, ra: 7, imm });
    assert_runs_as_split(&[
        (lwz(0), or(6, 5, 4, true)),
        (lwz(4), or(6, 5, 5, false)),
        (lwz(0x100), or(6, 5, 4, false)),
    ]);
}

#[test]
fn a_branch_into_the_second_slot_runs_it_alone() {
    let b_over_one = (18 << 26) | 8;
    let second_halves = [
        word(I::Oris {
            ra: 6,
            rs: 5,
            imm: 1,
        }),
        or(6, 5, 3, false),
        word(I::Lwz {
            rt: 6,
            ra: 7,
            imm: 0,
        }),
        word(I::Addi {
            rt: 6,
            ra: 5,
            imm: 1,
        }),
    ];
    let firsts = [
        word(I::Rldicr {
            ra: 5,
            rs: 4,
            sh: 8,
            me: 40,
            rc: false,
        }),
        or(5, 4, 3, false),
        or(5, 4, 3, false),
        word(I::Ld {
            rt: 5,
            ra: 1,
            imm: 0,
        }),
    ];
    for (first, second) in firsts.into_iter().zip(second_halves) {
        let code = [b_over_one, first, second, addi_r3()];
        assert!(fuses_at_4(&code), "{first:#010x} then {second:#010x}");
        let fused = run(&code, true, 64);
        assert_eq!(fused, run(&code, false, 64));
        assert_eq!(fused.retired, 3, "the first half did not run");
    }
}
