//! An atomic command whose effective address lies past the Cell's
//! real-address bound queues an MFC data-segment exception, and names no
//! reservation line there.

use crate::SpuExecutionUnit;
use cellgov_dma::MfcCommandError;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GETLLAR, MFC_PUTLLC};
use cellgov_time::Budget;

/// The first effective address past the Cell's real-address bound.
const PAST_THE_SPACE: u64 = CELL_EA_LIMIT + 1;

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> [u8; 4] {
    (0x081u32 << 23 | (imm << 7) | rt).to_be_bytes()
}

/// `wrch $chN, $rt`.
fn wrch(channel: u8, rt: u32) -> [u8; 4] {
    (0x10Du32 << 21 | (u32::from(channel) << 7) | rt).to_be_bytes()
}

/// Issues `cmd` with its effective address at `ea`, and returns the unit,
/// the step's yield and its effects.
fn issue(cmd: u32, ea: u64) -> (SpuExecutionUnit, YieldReason, Vec<Effect>) {
    let mut unit = SpuExecutionUnit::new(UnitId::new(3));
    let s = unit.state_mut();
    s.channels.mfc_eah = (ea >> 32) as u32;
    s.channels.mfc_eal = ea as u32;
    s.channels.mfc_lsa = 0x200;
    s.ls[0..4].copy_from_slice(&il(10, cmd));
    s.ls[4..8].copy_from_slice(&wrch(MFC_CMD, 10));
    let mem = GuestMemory::new(0x1000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (unit, result.yield_reason, effects)
}

/// [CBEA p:120 s:9.1.7] a segment fault suspends the queue and raises the MFC data-segment interrupt.
#[test]
fn an_atomic_command_past_the_address_space_queues_a_data_segment_exception() {
    for cmd in [MFC_GETLLAR, MFC_PUTLLC] {
        for ea in [PAST_THE_SPACE, u64::MAX - 0x7F] {
            let (unit, reason, effects) = issue(cmd, ea);
            assert_eq!(reason, YieldReason::DmaSubmitted, "0x{cmd:02x} at 0x{ea:x}");
            assert!(
                matches!(
                    effects.as_slice(),
                    [Effect::MfcInvalidCommand { command, .. }]
                        if command.error == MfcCommandError::DataSegment { ea }
                ),
                "0x{cmd:02x} at 0x{ea:x}: {effects:?}"
            );
            assert_ne!(unit.status(), UnitStatus::Faulted);
            assert!(unit.state().reservation().is_none());
        }
    }
}

/// The last line below the real-address bound names a segment. No region
/// backs it here, so the getllar queues a data-storage exception.
#[test]
fn the_last_line_of_the_space_is_no_segment_fault() {
    let (_, _, effects) = issue(MFC_GETLLAR, CELL_EA_LIMIT);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::MfcInvalidCommand { command, .. }]
                if command.error == MfcCommandError::DataStorage { ea: CELL_EA_LIMIT & !0x7F }
        ),
        "{effects:?}"
    );
}
