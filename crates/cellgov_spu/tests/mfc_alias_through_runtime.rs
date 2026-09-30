//! One SPU of a thread group puts data into another's local store
//! through the SPU thread window, then signals it with a fenced sndsig;
//! the other reads the signal and finds the data.

// [CBEA p:38 s:3.2.1] an MFC effective address can name another SPU's aliased local store.
// [CBEA p:72 s:7.9.4] sndsig writes another SPU's signal-notification register through its effective address.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAL, MFC_LSA, MFC_PUT, MFC_RD_TAG_STAT, MFC_SIZE, MFC_SNDSIGF, MFC_TAG_ID,
    MFC_TAG_UPDATE_ALL, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, SPU_RD_SIG_NOTIFY_1,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 2;
/// Slot 1's local store, where the second thread of the group sits.
const PEER_LS: u32 = 0xF010_0000;
/// Slot 1's SPU_Sig_Notify_1: the problem-state area at 0x40000, then
/// the register's offset in it.
const PEER_SIGNAL_1: u32 = PEER_LS + 0x4_0000 + 0x1_400C;

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

fn spu(id: UnitId, program: &[u32], regs: &[(u8, u32)]) -> SpuExecutionUnit {
    let mut spu = SpuExecutionUnit::new(id);
    let state = spu.state_mut();
    for (i, word) in program.iter().enumerate() {
        state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    for &(reg, value) in regs {
        state.set_reg_word_splat(reg, value);
    }
    spu
}

#[test]
fn a_put_and_a_signal_through_the_window_reach_the_peer_thread() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 200);
    let sender = rt.register_unit_with(|id| {
        let mut unit = spu(
            id,
            &[
                // put 16 bytes from 0x400 to the peer's 0x800
                wrch(MFC_LSA, 10),
                wrch(MFC_EAL, 11),
                wrch(MFC_SIZE, 12),
                wrch(MFC_TAG_ID, 13),
                wrch(MFC_CMD, 14),
                // sndsigf the word at 0x50C to the peer's signal 1
                wrch(MFC_LSA, 15),
                wrch(MFC_EAL, 16),
                wrch(MFC_SIZE, 17),
                wrch(MFC_TAG_ID, 13),
                wrch(MFC_CMD, 18),
                wrch(MFC_WR_TAG_MASK, 19),
                wrch(MFC_WR_TAG_UPDATE, 20),
                rdch(MFC_RD_TAG_STAT, 21),
                0,
            ],
            &[
                (10, 0x400),
                (11, PEER_LS + 0x800),
                (12, 16),
                (13, TAG),
                (14, MFC_PUT),
                (15, 0x50C),
                (16, PEER_SIGNAL_1),
                (17, 4),
                (18, MFC_SNDSIGF),
                (19, 1 << TAG),
                (20, MFC_TAG_UPDATE_ALL),
            ],
        );
        let ls = &mut unit.state_mut().ls;
        ls[0x400..0x410].copy_from_slice(&[0xA5; 16]);
        ls[0x50C..0x510].copy_from_slice(&0x1234u32.to_be_bytes());
        unit
    });
    let receiver = rt.register_unit_with(|id| spu(id, &[rdch(SPU_RD_SIG_NOTIFY_1, 3), 0], &[]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(2).expect("a group id");
    groups.record_spu(sender, group, 0).expect("slot 0");
    groups.record_spu(receiver, group, 1).expect("slot 1");

    let mut finished = Vec::new();
    for _ in 0..200 {
        let Ok(step) = rt.step() else { break };
        if step.result.yield_reason == YieldReason::Finished {
            finished.push(step.unit);
        }
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if finished.len() == 2 {
            break;
        }
    }
    assert_eq!(finished.len(), 2, "both threads stop: {finished:?}");
    assert_eq!(rt.take_mfc_exception(), None);

    let state = |unit| {
        rt.registry()
            .get(unit)
            .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
            .expect("an SPU unit")
            .state()
            .clone()
    };
    let peer = state(receiver);
    assert_eq!(peer.reg_word(3), 0x1234, "the signal arrived");
    assert_eq!(peer.ls[0x800..0x810], [0xA5; 16], "the data arrived first");
    assert_eq!(state(sender).reg_word(21), 1 << TAG);
}
