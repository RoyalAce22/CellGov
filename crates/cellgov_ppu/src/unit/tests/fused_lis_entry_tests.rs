//! A branch over `lis` into the `ori` it fused with runs the `ori` on the
//! rT it finds, as the unfused code does.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `lis r3, 0x1234`.
const LIS_R3: u32 = (15 << 26) | (3 << 21) | 0x1234;
/// `ori r3, r3, 0x00F0`.
const ORI_R3: u32 = (24 << 26) | (3 << 21) | (3 << 16) | 0x00F0;

/// r3 after `b +8; lis r3; <second>` retires two instructions from
/// r3 = 0x0000_0001_0000_0100.
fn r3_after_entry(second: u32) -> u64 {
    let code = [B_OVER_ONE, LIS_R3, second]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(8),
        Some(PpuInstruction::Consumed),
        "precondition: lis fuses with {second:#010x}"
    );
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).unwrap(),
        &code,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(3, 0x0000_0001_0000_0100);
    unit.run_until_yield(
        Budget::new(2),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(unit.retired_instructions(), 2, "the case ran to its end");
    unit.state().gpr[3]
}

/// [PPC-Book1 p:66 s:3.3.11] ori ORs UI into all 64 bits of RS.
#[test]
fn a_branch_into_the_ori_ors_into_the_r3_it_finds() {
    assert_eq!(r3_after_entry(ORI_R3), 0x0000_0001_0000_01F0);
}
