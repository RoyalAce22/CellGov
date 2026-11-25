//! An `sc` reaches the system call handler through a vector shaped like
//! the 4.93 kernel's.

use super::interrupt_support::*;
use cellgov_exec::YieldReason;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_CALL;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SPRG0, SPR_SRR0, SPR_SRR1};

/// The 4.93 kernel's system call handler, which its 0xC00 vector enters.
const HANDLER: u64 = 0x8000_0000_0027_5A90;
/// The real address that holds [`HANDLER`]'s effective address for the
/// stub's `ld`.
const HANDLER_ADDR: u64 = 0x1000;
/// `b .`.
const BRANCH_TO_SELF: u32 = 0x4800_0000;

/// The stub at the 0xC00 vector:
/// 1. save r3, r4 and r5 in SPRG1, SPRG2 and SPRG3;
/// 2. r3 = SRR0 and r4 = SRR1;
/// 3. SRR0 = the handler, SRR1 = the vector's real-mode MSR;
/// 4. `rfid`.
fn vector_stub() -> [u32; 10] {
    [
        mtspr(SPR_SPRG0 + 1, 3),
        mtspr(SPR_SPRG0 + 2, 4),
        mtspr(SPR_SPRG0 + 3, 5),
        mfspr(3, SPR_SRR0),
        mfspr(4, SPR_SRR1),
        ld(5, HANDLER_ADDR as u16, 0),
        mtspr(SPR_SRR0, 5),
        mfmsr(5),
        mtspr(SPR_SRR1, 5),
        RFID,
    ]
}

#[test]
fn the_instruction_at_the_system_call_vector_pays_no_lv2_round_trip() {
    let mut mem = memory(&[SC]);
    put_words(&mut mem, VECTOR_SYSTEM_CALL, &[NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, VECTOR_SYSTEM_CALL + 4);
    let cost = cellgov_time::GuestTicks::from(r.consumed_cost).raw();
    assert!(
        cost < cellgov_ps3_abi::hw::ppu_issue::PPU_SYSCALL_ROUND_TRIP_CYCLES,
        "the nop at the vector costs {cost} cycles"
    );
}

/// [PPC-Book3 p:73 s:5.5.13] A system call interrupt sets SRR0 to the instruction after `sc` and loads SRR1 from the MSR outside bits 33:36 and 42:47.
#[test]
fn an_sc_reaches_the_4_93_handler_with_the_callers_srr0_and_srr1_in_r3_and_r4() {
    let mut mem = memory(&[SC]);
    put_words(&mut mem, VECTOR_SYSTEM_CALL, &vector_stub());
    put(&mut mem, HANDLER_ADDR, &HANDLER.to_be_bytes());
    // Real mode ignores EA bits 0:1, so the handler runs at real 0x275A90.
    // Its first word branches to itself.
    put_words(&mut mem, HANDLER & 0x3F_FFFF, &[BRANCH_TO_SELF]);
    let mut unit = kernel_unit(KERNEL_MSR);
    for (r, v) in [(3, 0x33), (4, 0x44), (5, 0x55)] {
        unit.state_mut().set_gpr(r, v);
    }

    let (r, _) = run(&mut unit, &mem, 64);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().pc, VECTOR_SYSTEM_CALL);
    // A move that changes the MSR ends the batch, so the stub can take
    // more than one batch to reach the handler.
    for _ in 0..8 {
        if unit.state().pc == HANDLER {
            break;
        }
        let (r, _) = run(&mut unit, &mem, 64);
        assert_eq!(
            r.yield_reason,
            YieldReason::BudgetExhausted,
            "{:?}",
            r.local_diagnostics
        );
    }
    assert_eq!(unit.state().pc, HANDLER);
    let s = unit.state();
    assert_eq!(
        (s.gpr[3], s.gpr[4]),
        (CODE + 4, KERNEL_MSR),
        "r3 and r4 hold the caller's SRR0 and SRR1"
    );
    assert_eq!(sup(&unit).sprg[1..], [0x33, 0x44, 0x55]);
}
