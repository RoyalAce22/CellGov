//! A blocking channel access with a zero count parks the SPU on the
//! access, and only that channel's producer wakes it to run the access
//! again.

use cellgov_core::{Runtime, RuntimeMode};
use cellgov_event::UnitId;
use cellgov_exec::{UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_PUT, MFC_RD_TAG_STAT, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE, SPU_IN_MBOX_DEPTH,
    SPU_RD_IN_MBOX, SPU_WR_OUT_MBOX,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `il rt, imm`: RI16 opcode 0x081.
fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

/// `rdch rt, channel`: RR opcode 0x00D.
fn rdch(rt: u32, channel: u8) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `wrch channel, rt`: RR opcode 0x10D.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// A runtime with one SPU running `program` and an inbound mailbox,
/// staged for a 16-byte put under tag 1 that `r2` and `r7` start.
fn runtime_with(program: &[u32], mode: RuntimeMode) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(100), 400);
    rt.set_mode(mode);
    rt.mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let state = spu.state_mut();
        state.set_reg_word_splat(2, MFC_PUT);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.channels.mfc_lsa = 0x100;
        state.channels.mfc_eal = 0x1000;
        state.channels.mfc_size = 16;
        state.channels.mfc_tag_id = 1;
        state.channels.tag_mask = 1 << 1;
        spu
    });
    (rt, unit)
}

/// Runs one scheduled step and commits it; `None` when nothing is runnable.
fn step(rt: &mut Runtime) -> Option<YieldReason> {
    let step = rt.step().ok()?;
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    Some(step.result.yield_reason)
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

fn status(rt: &Runtime, unit: UnitId) -> Option<UnitStatus> {
    rt.registry().effective_status(unit)
}

/// [CBEA p:98 s:8.6.1] a write to a full outbound mailbox stalls the SPU until another processor reads it.
#[test]
fn a_second_outbound_message_waits_for_the_first_to_be_read() {
    let program = [
        il(4, 1),
        wrch(SPU_WR_OUT_MBOX, 4),
        il(4, 2),
        wrch(SPU_WR_OUT_MBOX, 4),
        0,
    ];
    let (mut rt, unit) = runtime_with(&program, RuntimeMode::FullTrace);
    assert_eq!(step(&mut rt), Some(YieldReason::ChannelStall));
    assert_eq!(
        spu(&rt, unit).state().pc,
        12,
        "the second write did not retire"
    );
    assert_eq!(status(&rt, unit), Some(UnitStatus::Blocked));
    assert_eq!(step(&mut rt), None, "nothing wakes the writer yet");

    assert_eq!(rt.read_unit_out_mbox(unit), Ok(Some(1)));
    assert_eq!(status(&rt, unit), Some(UnitStatus::Runnable));
    assert_eq!(step(&mut rt), Some(YieldReason::Finished));
    assert_eq!(rt.read_unit_out_mbox(unit), Ok(Some(2)));
}

/// [CBE-Handbook p:447 s:17.1.6] a blocked access stalls until the channel changes.
#[test]
fn a_dma_completion_does_not_wake_a_mailbox_read() {
    // A put, then a read of the empty inbound mailbox.
    let program = [wrch(MFC_CMD, 2), rdch(5, SPU_RD_IN_MBOX), 0];
    let (mut rt, unit) = runtime_with(&program, RuntimeMode::FullTrace);
    assert_eq!(step(&mut rt), Some(YieldReason::DmaSubmitted));
    assert_eq!(step(&mut rt), Some(YieldReason::ChannelStall));
    assert_eq!(
        step(&mut rt),
        None,
        "the put lands, and the read stays parked on its own channel"
    );
    assert_eq!(status(&rt, unit), Some(UnitStatus::Blocked));
    assert_eq!(spu(&rt, unit).state().pc, 4);

    rt.write_unit_in_mbox(unit, 0x77)
        .expect("the SPU has a mailbox");
    assert_eq!(step(&mut rt), Some(YieldReason::MailboxAccess));
    assert_eq!(spu(&rt, unit).state().reg_word(5), 0x77);
}

/// [CBE-Handbook p:447 s:17.1.6] a blocked access stalls until the channel changes.
#[test]
fn a_mailbox_message_does_not_wake_a_tag_status_read() {
    let program = [
        wrch(MFC_CMD, 2),
        wrch(MFC_WR_TAG_UPDATE, 7),
        rdch(5, MFC_RD_TAG_STAT),
        0,
    ];
    let (mut rt, unit) = runtime_with(&program, RuntimeMode::FullTrace);
    assert_eq!(step(&mut rt), Some(YieldReason::DmaSubmitted));
    assert_eq!(step(&mut rt), Some(YieldReason::ChannelStall));
    rt.write_unit_in_mbox(unit, 0x77)
        .expect("the SPU has a mailbox");
    assert_eq!(
        status(&rt, unit),
        Some(UnitStatus::Blocked),
        "a message is not the tag status the read waits for"
    );
    // The put's completion ends the wait, and the read runs again.
    let mut last = None;
    while let Some(reason) = step(&mut rt) {
        last = Some(reason);
    }
    assert_eq!(last, Some(YieldReason::Finished));
    assert_eq!(spu(&rt, unit).state().reg_word(5), 1 << 1);
}

/// Two waiting messages leave in order, one read per step.
#[test]
fn two_reads_take_two_messages_in_order() {
    let program = [rdch(5, SPU_RD_IN_MBOX), rdch(6, SPU_RD_IN_MBOX), 0];
    let (mut rt, unit) = runtime_with(&program, RuntimeMode::FullTrace);
    rt.write_unit_in_mbox(unit, 0xA).expect("a message");
    rt.write_unit_in_mbox(unit, 0xB).expect("a message");
    assert_eq!(step(&mut rt), Some(YieldReason::MailboxAccess));
    assert_eq!(step(&mut rt), Some(YieldReason::MailboxAccess));
    assert_eq!(step(&mut rt), Some(YieldReason::Finished));
    let state = spu(&rt, unit).state();
    assert_eq!((state.reg_word(5), state.reg_word(6)), (0xA, 0xB));
}

/// The fault-driven mode skips per-step arbitration for most yields; a
/// channel stall still parks the unit.
#[test]
fn a_channel_stall_parks_in_the_fault_driven_mode() {
    let program = [rdch(5, SPU_RD_IN_MBOX), 0];
    let (mut rt, unit) = runtime_with(&program, RuntimeMode::FaultDriven);
    assert_eq!(step(&mut rt), Some(YieldReason::ChannelStall));
    assert_eq!(status(&rt, unit), Some(UnitStatus::Blocked));
    assert_eq!(step(&mut rt), None);
}
