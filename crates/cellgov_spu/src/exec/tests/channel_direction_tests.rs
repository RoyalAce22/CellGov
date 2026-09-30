//! A channel instruction in the wrong direction stops the SPU with an
//! invalid-channel stop and changes nothing else.

// [CBEA p:109 s:9] issuing a channel instruction inappropriate for the channel's definition results in an invalid channel instruction interrupt.

use super::*;
use crate::state::{SpuObservableSnapshot, SpuState};
use crate::stop::SpuStopKind;
use cellgov_ps3_abi::hw::spu::{channel_direction, ChannelDirection};

/// The read and read-blocking channels.
///
/// [CBEA p:299 s:Appendix B, Table B-1], [CBEA p:300 s:Appendix B, Table B-1], [CBEA p:301 s:Appendix B, Table B-1] the access type of each channel.
const READ: [u8; 12] = [
    0x00, 0x03, 0x04, 0x08, 0x0B, 0x0C, 0x0D, 0x0F, 0x18, 0x19, 0x1B, 0x1D,
];
/// The write and write-blocking channels.
const WRITE: [u8; 16] = [
    0x01, 0x02, 0x07, 0x09, 0x0E, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x1A, 0x1C, 0x1E,
];

const INVALID_CHANNEL: SpuStepOutcome = SpuStepOutcome::Stop {
    kind: SpuStopKind::InvalidChannel,
    signal: 0,
};

#[test]
fn the_direction_table_matches_the_channel_map() {
    for channel in 0..=0x7F {
        let expected = if READ.contains(&channel) {
            Some(ChannelDirection::Read)
        } else if WRITE.contains(&channel) {
            Some(ChannelDirection::Write)
        } else {
            None
        };
        assert_eq!(
            channel_direction(channel),
            expected,
            "channel 0x{channel:02x}"
        );
    }
}

#[test]
fn wrch_to_a_read_channel_is_an_invalid_channel_stop() {
    for channel in READ {
        let mut s = SpuState::new();
        s.set_reg(7, [0xA5; 16]);
        let before = SpuObservableSnapshot::capture(&s);
        let out = execute(
            &SpuInstruction::Wrch { channel, rt: 7 },
            &mut s,
            UnitId::new(0),
        );
        assert_eq!(out, INVALID_CHANNEL, "channel 0x{channel:02x}");
        assert_eq!(
            SpuObservableSnapshot::capture(&s),
            before,
            "channel 0x{channel:02x}"
        );
    }
}

#[test]
fn rdch_of_a_write_channel_is_an_invalid_channel_stop() {
    for channel in WRITE {
        let mut s = SpuState::new();
        s.set_reg(7, [0xA5; 16]);
        let before = SpuObservableSnapshot::capture(&s);
        let out = execute(
            &SpuInstruction::Rdch { rt: 7, channel },
            &mut s,
            UnitId::new(0),
        );
        assert_eq!(out, INVALID_CHANNEL, "channel 0x{channel:02x}");
        assert_eq!(
            SpuObservableSnapshot::capture(&s),
            before,
            "channel 0x{channel:02x}"
        );
    }
}

#[test]
fn rchcnt_is_never_in_the_wrong_direction() {
    for channel in READ.into_iter().chain(WRITE) {
        let mut s = SpuState::new();
        let out = execute(
            &SpuInstruction::Rchcnt { rt: 7, channel },
            &mut s,
            UnitId::new(0),
        );
        assert_ne!(out, INVALID_CHANNEL, "channel 0x{channel:02x}");
    }
}

/// `rdch r7, MFC_LSA`: RR opcode 0x00D.
const RDCH_R7_MFC_LSA: u32 = (0x00D << 21) | (0x10 << 7) | 7;
/// `nop`: RR opcode 0x201.
const NOP: u32 = 0x201 << 21;

/// [CBEA p:93 s:8.5.2] `SPU_Status[C]`: an invalid channel instruction was detected and the SPU stopped.
#[test]
fn a_unit_that_meets_a_wrong_direction_rdch_stops_with_c_on_that_word() {
    use crate::stop::SpuStop;
    use crate::SpuExecutionUnit;
    use cellgov_exec::{ExecutionContext, ExecutionUnit, StopRegisters, UnitStatus, YieldReason};
    use cellgov_mem::GuestMemory;
    use cellgov_ps3_abi::hw::spu::SPU_STATUS_C;
    use cellgov_time::Budget;

    let mut unit = SpuExecutionUnit::new(UnitId::new(5));
    for (i, word) in [NOP, NOP, RDCH_R7_MFC_LSA].iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    let mem = GuestMemory::new(0x1000);
    let result = unit.run_until_yield(
        Budget::new(100),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(result.fault, None);
    assert_eq!(unit.status(), UnitStatus::Finished);
    assert_eq!(
        unit.state().stop,
        Some(SpuStop {
            kind: SpuStopKind::InvalidChannel,
            code: 0,
            npc: 8,
            interrupts_enabled: false,
        })
    );
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: SPU_STATUS_C,
            npc: 8
        })
    );
}
