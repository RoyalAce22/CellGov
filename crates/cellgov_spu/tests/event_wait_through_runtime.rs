//! An SPU waits on an event through `SPU_RdEventStat`, and each event
//! source the model raises ends the wait: the inbound mailbox, a signal
//! notification, the tag-group status update, the multisource
//! synchronization, and the loss of a lock-line reservation.

// [CBEA p:147 s:9.11.1] a read of SPU_RdEventStat with count 0 stalls until an enabled event occurs.
// [CBEA p:149 s:9.11.1], [CBEA p:150 s:9.11.1] each event is the edge of its source channel's count from 0 to nonzero.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_exec::{SignalNotifier, StallWake, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    event, MFC_CMD, MFC_EAL, MFC_GET, MFC_GETLLAR, MFC_LSA, MFC_PUT, MFC_PUTLLC, MFC_PUTLLUC,
    MFC_RD_ATOMIC_STAT, MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_MSSYNC_REQ,
    MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, SPU_IN_MBOX_DEPTH, SPU_RD_EVENT_STAT, SPU_RD_IN_MBOX,
    SPU_RD_SRR0, SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `wrch $ch<channel>, rt`.
const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

/// `rchcnt rt, $ch<channel>`.
const fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | ((channel as u32) << 7) | rt
}

/// A runtime holding one SPU that runs `program` with `regs` set, and
/// an inbound mailbox beside it.
fn runtime(program: &[u32], regs: &[(u8, u32)]) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 200);
    let unit = add_spu(&mut rt, program, regs);
    (rt, unit)
}

/// Registers an SPU that runs `program` with `regs` set, and its
/// inbound mailbox.
fn add_spu(rt: &mut Runtime, program: &[u32], regs: &[(u8, u32)]) -> UnitId {
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    rt.register_unit_with(|id| {
        assert_eq!(id.raw(), mailbox.raw(), "the SPU's mailbox shares its id");
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        for &(reg, value) in regs {
            state.set_reg_word_splat(reg, value);
        }
        spu
    })
}

fn step(rt: &mut Runtime) {
    let step = rt.step().expect("a unit runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
}

/// Steps until the SPU stops and returns its register 5.
fn finish(rt: &mut Runtime, unit: UnitId) -> u32 {
    for _ in 0..40 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished)
    );
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state()
        .reg_word(5)
}

fn parked_on_an_event(rt: &Runtime, unit: UnitId) -> bool {
    rt.registry().effective_status(unit) == Some(UnitStatus::Blocked)
        && rt
            .registry()
            .get(unit)
            .and_then(|unit| unit.channel_stall())
            .map(|stall| stall.wake)
            == Some(StallWake::Event)
}

/// Enable the events in register 10, wait, stop.
const WAIT: [u32; 3] = [wrch(SPU_WR_EVENT_MASK, 10), rdch(SPU_RD_EVENT_STAT, 5), 0];

/// [CBEA p:150 s:9.11.1] Mb is set when the SPU_RdInMbox count changes from 0 to nonzero.
#[test]
fn a_mailbox_write_ends_a_wait_on_mb() {
    let (mut rt, unit) = runtime(&WAIT, &[(10, event::MB)]);
    step(&mut rt);
    assert!(parked_on_an_event(&rt, unit));
    rt.write_unit_in_mbox(unit, 7).expect("problem state");
    assert_eq!(finish(&mut rt, unit), event::MB);
}

/// [CBEA p:150 s:9.11.1] S1 is set when the SPU Signal Notification 1 count changes from 0 to nonzero.
#[test]
fn a_signal_write_ends_a_wait_on_s1_and_a_masked_one_does_not() {
    let (mut rt, unit) = runtime(&WAIT, &[(10, event::S1)]);
    step(&mut rt);
    rt.write_unit_signal(unit, SignalNotifier::Two, 5)
        .expect("problem state");
    for _ in 0..3 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert!(
        parked_on_an_event(&rt, unit),
        "S2 is masked, so the SPU waits again"
    );
    rt.write_unit_signal(unit, SignalNotifier::One, 9)
        .expect("problem state");
    assert_eq!(finish(&mut rt, unit), event::S1);
}

/// [CBEA p:149 s:9.11.1] Tg is set when the MFC_RdTagStat count changes from 0 to nonzero.
#[test]
fn a_tag_status_update_ends_a_wait_on_tg() {
    let program = [
        wrch(MFC_LSA, 11),
        wrch(MFC_EAL, 12),
        wrch(MFC_SIZE, 13),
        wrch(MFC_TAG_ID, 14),
        wrch(MFC_CMD, 15),
        wrch(MFC_WR_TAG_MASK, 16),
        wrch(MFC_WR_TAG_UPDATE, 17),
        wrch(SPU_WR_EVENT_MASK, 10),
        rdch(SPU_RD_EVENT_STAT, 5),
        0,
    ];
    let (mut rt, unit) = runtime(
        &program,
        &[
            (10, event::TG),
            (11, 0x800),
            (12, 0x100),
            (13, 16),
            (14, 3),
            (15, MFC_GET),
            (16, 1 << 3),
            (17, MFC_TAG_UPDATE_ALL),
        ],
    );
    let mut parked = false;
    for _ in 0..40 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        parked |= parked_on_an_event(&rt, unit);
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert!(parked, "the SPU waited for the get");
    assert_eq!(finish(&mut rt, unit), event::TG);
}

/// [CBEA p:148 s:9.11.1] a MFC_WrMSSyncReq write with no transfer pending triggers Ms at once.
#[test]
fn a_multisource_request_with_nothing_in_flight_ends_a_wait_on_ms_at_once() {
    let program = [
        wrch(SPU_WR_EVENT_MASK, 10),
        wrch(MFC_WR_MSSYNC_REQ, 0),
        rdch(SPU_RD_EVENT_STAT, 5),
        0,
    ];
    let (mut rt, unit) = runtime(&program, &[(10, event::MS)]);
    let first = rt.step().expect("the SPU runs");
    assert_ne!(first.result.yield_reason, YieldReason::ChannelStall);
    rt.commit_step(&first.result, &first.effects)
        .expect("the step commits");
    assert_eq!(finish(&mut rt, unit), event::MS);
}

/// The line every Lr test reserves, and one beside it.
const LINE_A: u32 = 0x100;
const LINE_B: u32 = 0x180;

/// Enable Lr, reserve the line in register 12, wait, stop.
const RESERVE_AND_WAIT: [u32; 7] = [
    wrch(SPU_WR_EVENT_MASK, 10),
    wrch(MFC_LSA, 11),
    wrch(MFC_EAL, 12),
    wrch(MFC_CMD, 13),
    rdch(MFC_RD_ATOMIC_STAT, 6),
    rdch(SPU_RD_EVENT_STAT, 5),
    0,
];

const RESERVER_REGS: [(u8, u32); 4] = [
    (10, event::LR),
    (11, 0x800),
    (12, LINE_A),
    (13, MFC_GETLLAR),
];

fn place(rt: &mut Runtime, addr: u32) {
    let range = ByteRange::new(GuestAddr::new(u64::from(addr)), 4).expect("a small range");
    rt.place_bytes(AddressSpaceId::BOOT, range, &[0x5A; 4])
        .expect("the placement lands");
}

/// [CBEA p:148 s:9.11.1] Lr is set when a snoop external to the MFC resets the reservation.
/// [CBEA p:67 s:7.8.1] a program can wait on the event with a read of SPU_RdEventStat instead of issuing getllar again.
#[test]
fn another_spus_putlluc_into_the_line_ends_a_wait_on_lr() {
    let (mut rt, waiter) = runtime(&RESERVE_AND_WAIT, &RESERVER_REGS);
    add_spu(
        &mut rt,
        &[
            wrch(MFC_LSA, 11),
            wrch(MFC_EAL, 12),
            wrch(MFC_CMD, 13),
            rdch(MFC_RD_ATOMIC_STAT, 6),
            0,
        ],
        &[(11, 0x800), (12, LINE_A), (13, MFC_PUTLLUC)],
    );
    step(&mut rt);
    assert!(parked_on_an_event(&rt, waiter), "the waiter holds the line");
    assert_eq!(finish(&mut rt, waiter), event::LR);
}

/// [CBEA p:164 s:9.12.10] the reservation is lost when another processor or device modifies the line.
#[test]
fn a_host_placement_into_the_line_ends_a_wait_on_lr() {
    let (mut rt, waiter) = runtime(&RESERVE_AND_WAIT, &RESERVER_REGS);
    step(&mut rt);
    assert!(parked_on_an_event(&rt, waiter));
    place(&mut rt, LINE_A + 0x7C);
    assert_eq!(finish(&mut rt, waiter), event::LR);
}

#[test]
fn a_placement_beside_the_line_leaves_the_wait_parked() {
    let (mut rt, waiter) = runtime(&RESERVE_AND_WAIT, &RESERVER_REGS);
    step(&mut rt);
    place(&mut rt, LINE_B);
    assert!(parked_on_an_event(&rt, waiter));
    assert!(rt.step().is_err(), "nothing can run");
}

/// [CBEA p:148 s:9.11.1] Lr is not set for a reservation reset by a local action.
/// [CBEA p:67 s:7.8.1] a getllar to another line can lose the existing reservation; the local-action rule governs it.
#[test]
fn putllc_getllar_to_another_line_and_putlluc_raise_no_lr() {
    let program = [
        wrch(SPU_WR_EVENT_MASK, 10),
        wrch(MFC_LSA, 11),
        wrch(MFC_EAL, 12),
        wrch(MFC_CMD, 13),
        rdch(MFC_RD_ATOMIC_STAT, 6),
        wrch(MFC_CMD, 14),
        rdch(MFC_RD_ATOMIC_STAT, 6),
        rchcnt(SPU_RD_EVENT_STAT, 20),
        wrch(MFC_CMD, 13),
        rdch(MFC_RD_ATOMIC_STAT, 6),
        wrch(MFC_EAL, 15),
        wrch(MFC_CMD, 13),
        rdch(MFC_RD_ATOMIC_STAT, 6),
        rchcnt(SPU_RD_EVENT_STAT, 21),
        wrch(MFC_CMD, 16),
        rdch(MFC_RD_ATOMIC_STAT, 6),
        rchcnt(SPU_RD_EVENT_STAT, 22),
        0,
    ];
    let (mut rt, unit) = runtime(
        &program,
        &[
            (10, event::LR),
            (11, 0x800),
            (12, LINE_A),
            (13, MFC_GETLLAR),
            (14, MFC_PUTLLC),
            (15, LINE_B),
            (16, MFC_PUTLLUC),
            (20, 0xFF),
            (21, 0xFF),
            (22, 0xFF),
        ],
    );
    finish(&mut rt, unit);
    let state = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state();
    assert_eq!(state.reg_word(20), 0, "after putllc");
    assert_eq!(state.reg_word(21), 0, "after a getllar to another line");
    assert_eq!(state.reg_word(22), 0, "after putlluc");
}

/// [CBEA p:164 s:9.12.10] the reservation is lost when another processor or device modifies the line.
#[test]
fn a_put_landing_in_the_line_while_every_unit_waits_ends_a_wait_on_lr() {
    let (mut rt, waiter) = runtime(&RESERVE_AND_WAIT, &RESERVER_REGS);
    add_spu(
        &mut rt,
        &[
            wrch(MFC_LSA, 11),
            wrch(MFC_EAL, 12),
            wrch(MFC_SIZE, 13),
            wrch(MFC_TAG_ID, 14),
            wrch(MFC_CMD, 15),
            0,
        ],
        &[(11, 0x800), (12, LINE_A), (13, 16), (14, 0), (15, MFC_PUT)],
    );
    step(&mut rt);
    step(&mut rt);
    assert!(
        parked_on_an_event(&rt, waiter),
        "the put is still in flight, and the other SPU stopped"
    );
    assert_eq!(finish(&mut rt, waiter), event::LR);
}

/// [CBE-Handbook p:447 s:17.1.6] a blocked channel access stalls until the channel changes or the SPU is interrupted.
/// [SPU-ISA p:251 s:12.1] the interrupt saves the address of the next instruction, the stalled read, in SRR0.
#[test]
fn an_interrupt_ends_a_stalled_mailbox_read() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 200);
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        assert_eq!(id.raw(), mailbox.raw());
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let handler = [rdch(SPU_RD_SRR0, 20), 0];
        let main = [rdch(SPU_RD_IN_MBOX, 5), 0];
        for (base, words) in [(0usize, &handler), (0x100, &main)] {
            for (i, word) in words.iter().enumerate() {
                state.ls[base + i * 4..base + i * 4 + 4].copy_from_slice(&word.to_be_bytes());
            }
        }
        state.pc = 0x100;
        state.channels.set_event_state(0, event::S1);
        state.interrupts_enabled = true;
        spu
    });
    step(&mut rt);
    assert!(
        parked_on_an_event(&rt, unit),
        "the mailbox read parks on any event"
    );
    rt.write_unit_signal(unit, SignalNotifier::One, 9)
        .expect("problem state");
    finish(&mut rt, unit);
    let state = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state();
    assert_eq!(state.reg_word(20), 0x100, "SRR0 names the stalled read");
    assert!(state.channels.in_mbox.is_empty());
}
