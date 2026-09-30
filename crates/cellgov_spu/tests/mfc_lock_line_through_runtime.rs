//! One SPU holds a reservation on a line; another publishes the line with
//! an unconditional lock-line put, then signals the first. The first
//! SPU's conditional store then fails and the published bytes stand.

// [CBEA p:67 s:7.8.3] putlluc stores whether or not a reservation exists.
// [CBEA p:68 s:7.8.4] putqlluc is putlluc placed in the command queue, completing through its tag group.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_ATOMIC_STAT_G, MFC_ATOMIC_STAT_S, MFC_ATOMIC_STAT_U, MFC_CMD, MFC_EAL, MFC_GETLLAR,
    MFC_LSA, MFC_PUTLLC, MFC_PUTLLUC, MFC_PUTQLLUC, MFC_RD_ATOMIC_STAT, MFC_RD_TAG_STAT, MFC_SIZE,
    MFC_SNDSIGF, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE,
    SPU_RD_SIG_NOTIFY_1,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 6;
/// The line both SPUs name.
const LINE: u32 = 0x100;
/// Slot 0's SPU_Sig_Notify_1 in the thread window.
const HOLDER_SIGNAL_1: u32 = 0xF000_0000 + 0x4_0000 + 0x1_400C;

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

fn state(rt: &Runtime, unit: UnitId) -> cellgov_spu::state::SpuState {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit")
        .state()
        .clone()
}

/// The holder's and the publisher's final states, and the line in
/// memory, once `publish` published over the holder's reservation.
fn publish_over_a_reservation(
    publish: u32,
) -> (
    cellgov_spu::state::SpuState,
    cellgov_spu::state::SpuState,
    Vec<u8>,
) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 200);
    let holder = rt.register_unit_with(|id| {
        spu(
            id,
            &[
                wrch(MFC_LSA, 10),
                wrch(MFC_EAL, 11),
                wrch(MFC_CMD, 12),
                rdch(MFC_RD_ATOMIC_STAT, 20),
                rdch(SPU_RD_SIG_NOTIFY_1, 3),
                wrch(MFC_LSA, 10),
                wrch(MFC_EAL, 11),
                wrch(MFC_CMD, 13),
                rdch(MFC_RD_ATOMIC_STAT, 21),
                0,
            ],
            &[(10, 0x400), (11, LINE), (12, MFC_GETLLAR), (13, MFC_PUTLLC)],
        )
    });
    let mut program = vec![
        wrch(MFC_LSA, 10),
        wrch(MFC_EAL, 11),
        wrch(MFC_TAG_ID, 14),
        wrch(MFC_CMD, 12),
    ];
    if publish == MFC_PUTLLUC {
        program.push(rdch(MFC_RD_ATOMIC_STAT, 20));
    }
    program.extend([
        // A fenced sndsig of the word at 0x50C to the holder's signal 1.
        wrch(MFC_LSA, 15),
        wrch(MFC_EAL, 16),
        wrch(MFC_SIZE, 17),
        wrch(MFC_TAG_ID, 14),
        wrch(MFC_CMD, 18),
        wrch(MFC_WR_TAG_MASK, 19),
        wrch(MFC_WR_TAG_UPDATE, 22),
        rdch(MFC_RD_TAG_STAT, 23),
        0,
    ]);
    let publisher = rt.register_unit_with(|id| {
        let mut unit = spu(
            id,
            &program,
            &[
                (10, 0x800),
                (11, LINE),
                (12, publish),
                (14, TAG),
                (15, 0x50C),
                (16, HOLDER_SIGNAL_1),
                (17, 4),
                (18, MFC_SNDSIGF),
                (19, 1 << TAG),
                (22, MFC_TAG_UPDATE_ALL),
            ],
        );
        let ls = &mut unit.state_mut().ls;
        ls[0x800..0x880].fill(0x77);
        ls[0x50C..0x510].copy_from_slice(&1u32.to_be_bytes());
        unit
    });
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let group = groups.create(2).expect("a group id");
    groups.record_spu(holder, group, 0).expect("slot 0");
    groups.record_spu(publisher, group, 1).expect("slot 1");

    let mut finished = 0;
    for _ in 0..400 {
        let Ok(step) = rt.step() else { break };
        finished += usize::from(step.result.yield_reason == YieldReason::Finished);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if finished == 2 {
            break;
        }
    }
    assert_eq!(finished, 2, "both SPUs stop");
    assert_eq!(rt.take_mfc_exception(), None);
    let line = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(u64::from(LINE)), 128).expect("in range"))
        .expect("readable")
        .to_vec();
    (state(&rt, holder), state(&rt, publisher), line)
}

#[test]
fn putlluc_publishes_over_another_unit_reservation_and_reports_u() {
    let (holder, publisher, line) = publish_over_a_reservation(MFC_PUTLLUC);
    assert_eq!(
        holder.reg_word(20),
        MFC_ATOMIC_STAT_G,
        "the holder reserved"
    );
    assert_eq!(holder.reg_word(3), 1, "the signal arrived");
    assert_eq!(
        holder.reg_word(21),
        MFC_ATOMIC_STAT_S,
        "the conditional store lost the line"
    );
    assert_eq!(publisher.reg_word(20), MFC_ATOMIC_STAT_U);
    assert_eq!(line, [0x77; 128], "the published line stands");
}

#[test]
fn putqlluc_publishes_through_its_tag_group() {
    let (holder, publisher, line) = publish_over_a_reservation(MFC_PUTQLLUC);
    assert_eq!(holder.reg_word(20), MFC_ATOMIC_STAT_G);
    assert_eq!(holder.reg_word(3), 1);
    assert_eq!(holder.reg_word(21), MFC_ATOMIC_STAT_S);
    assert_eq!(publisher.reg_word(23), 1 << TAG, "the tag group completed");
    assert!(
        !publisher.channels.atomic_status_ready,
        "a queued put reports no atomic status"
    );
    assert_eq!(line, [0x77; 128]);
}
