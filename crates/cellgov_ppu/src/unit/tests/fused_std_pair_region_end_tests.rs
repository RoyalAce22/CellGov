//! A `std; std` pair whose second doubleword runs past the end of its
//! region. The fused pair refuses and stages neither doubleword. The
//! batch then runs the pair split, so the first `std` commits and the
//! second faults at its own address, as the unfused stores do.
//!
//! [PPC-Book3 p:61 s:5.2] when an interrupt occurs, every preceding
//! instruction has completed and no subsequent instruction has begun.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

/// The end of the code region, at 0.
const CODE_END: u64 = 0x1000;
/// The base of the data region.
const DATA_BASE: u64 = 0x1_0000;
/// The end of the data region. No region holds this byte.
const DATA_END: u64 = 0x2_0000;
/// `r19`, the base of both stores.
const BASE: u64 = 0x1_FF40;
/// `r0`, the doubleword both stores write. No byte of it is 0, so each
/// byte a store commits differs from the zeroed region.
const VALUE: u64 = 0x0102_0304_0506_0708;
/// The first doubleword, which the region holds.
const FIRST_EA: u64 = BASE + 180;
/// The second doubleword. Its last four bytes lie past [`DATA_END`].
const SECOND_EA: u64 = BASE + 188;
const _: () = assert!(FIRST_EA + 8 <= DATA_END && SECOND_EA + 8 > DATA_END);

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `std r0, 180(r19)`.
const STD_FIRST: u32 = (62 << 26) | (19 << 16) | 180;
/// `std r0, 188(r19)`.
const STD_SECOND: u32 = (62 << 26) | (19 << 16) | 188;

const CODE: [u32; 4] = [ADDI_R3, STD_FIRST, STD_SECOND, ADDI_R3];

/// Where the unit stops, and the data region's last sixteen bytes.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    faulting_ea: Option<u64>,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    tail: Vec<u8>,
}

fn code_bytes() -> Vec<u8> {
    CODE.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// Runs [`CODE`], through the fused shadow or by plain decode, in
/// batches of `budget` until a batch faults, for at most eight batches.
/// Each batch that ends otherwise commits its stores.
fn run(fused: bool, budget: u64) -> Stop {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, CODE_END as usize, "code", PageSize::Page4K),
        Region::new(
            DATA_BASE,
            (DATA_END - DATA_BASE) as usize,
            "data",
            PageSize::Page4K,
        ),
    ])
    .unwrap();
    let code = code_bytes();
    let range = ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap();
    mem.apply_commit(range, &code).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &code));
    }
    unit.state_mut().set_gpr(0, VALUE);
    unit.state_mut().set_gpr(19, BASE);
    let mut last = None;
    for _ in 0..8 {
        let ctx = ExecutionContext::new(&mem);
        let mut effects = Vec::new();
        let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
        let fault = r.yield_reason == YieldReason::Fault;
        if !fault {
            for effect in effects {
                if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                    mem.apply_commit(range, bytes.bytes()).unwrap();
                }
            }
        }
        last = Some((
            r.yield_reason,
            r.local_diagnostics.pc,
            r.local_diagnostics.faulting_ea,
        ));
        if fault {
            break;
        }
    }
    let (reason, pc, faulting_ea) = last.expect("one batch ran");
    let tail = ByteRange::new(GuestAddr::new(DATA_END - 16), 16).unwrap();
    Stop {
        reason,
        pc,
        faulting_ea,
        retired: unit.retired_instructions(),
        state: unit.state().fingerprint(),
        tail: mem.read(tail).expect("the region holds its tail").to_vec(),
    }
}

#[test]
fn a_std_pair_whose_second_doubleword_straddles_the_region_end_commits_the_first() {
    let slot = shadow::PredecodedShadow::build(0, &code_bytes()).get(4);
    assert!(
        matches!(
            slot,
            Some(instruction::PpuInstruction::Fused(
                instruction::FusedPair::StdStd { .. }
            ))
        ),
        "precondition: std; std fuses at 0x4, got {slot:?}"
    );

    for budget in [1, 2, 3, 256] {
        assert_eq!(run(true, budget), run(false, budget), "budget {budget}");
    }

    let plain = run(false, 256);
    assert_eq!(plain.reason, YieldReason::Fault);
    assert_eq!(
        plain.pc,
        Some(8),
        "the second std faults at its own address"
    );
    assert_eq!(plain.faulting_ea, Some(DATA_END));
    assert_eq!(plain.retired, 2, "addi and the first std retired");
    let first = (FIRST_EA - (DATA_END - 16)) as usize;
    assert_eq!(
        plain.tail[first..first + 8],
        VALUE.to_be_bytes(),
        "the first doubleword committed"
    );
    assert_eq!(
        plain.tail[first + 8..],
        [0; 4],
        "no byte of the second doubleword committed"
    );
}
