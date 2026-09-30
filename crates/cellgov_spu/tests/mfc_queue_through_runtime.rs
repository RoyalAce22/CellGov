//! The MFC SPU command queue through the runtime: a get or put holds a
//! slot until it completes, a get's bytes land in local store when it
//! completes, and a write to a full queue parks until a slot frees.

use cellgov_core::{Runtime, RuntimeMode};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, StallWake, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_GET, MFC_PUT, MFC_RD_TAG_STAT, MFC_SPU_QUEUE_DEPTH, MFC_TAG_UPDATE_ALL,
    MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// A second region, clear of the base one, so a get reaches a region
/// other than the one at address 0.
const AUX_BASE: u64 = 0x10000;
const AUX_EA: u64 = AUX_BASE + 0x40;
const UNMAPPED_EA: u64 = 0x9_0000;
const TRANSFER_BYTES: u32 = 64;
const LSA: u32 = 0x200;
const TAG: u32 = 3;
/// The byte that fills the get's source.
const MARK: u8 = 0xA7;

/// `rchcnt rt, channel`: RR opcode 0x00F.
fn rchcnt(rt: u32, channel: u8) -> u32 {
    (0x00F << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, channel`: RR opcode 0x00D.
fn rdch(rt: u32, channel: u8) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `wrch channel, rt`: RR opcode 0x10D.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// Guest memory with the second region, its transfer bytes set to MARK.
fn memory() -> GuestMemory {
    let mut mem = GuestMemory::new(0x2000);
    mem.install_region(AUX_BASE, 0x1000, "aux", PageSize::Page64K)
        .expect("the auxiliary region is clear of the base region");
    let source =
        ByteRange::new(GuestAddr::new(AUX_EA), u64::from(TRANSFER_BYTES)).expect("a 64-byte range");
    mem.apply_commit(source, &[MARK; TRANSFER_BYTES as usize])
        .expect("the auxiliary region is writable");
    mem
}

/// A runtime with one SPU running `program`, its MFC channels naming a
/// transfer of `size` bytes between local store `lsa` and `ea`, with
/// `r2` = `command` and `r7` = an all-groups tag update request.
fn runtime_with(program: &[u32], command: u32, ea: u64, lsa: u32, size: u32) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(memory(), Budget::new(100), 400);
    rt.set_mode(RuntimeMode::FullTrace);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let state = spu.state_mut();
        state.set_reg_word_splat(2, command);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.channels.mfc_lsa = lsa;
        state.channels.mfc_eah = (ea >> 32) as u32;
        state.channels.mfc_eal = ea as u32;
        state.channels.mfc_size = size;
        state.channels.mfc_tag_id = TAG;
        state.channels.tag_mask = 1 << TAG;
        spu
    });
    (rt, unit)
}

/// Steps and commits until nothing is runnable; returns every yield.
fn run(rt: &mut Runtime) -> Vec<YieldReason> {
    let mut reasons = Vec::new();
    while let Ok(step) = rt.step() {
        reasons.push(step.result.yield_reason);
        if rt.commit_step(&step.result, &step.effects).is_err() {
            break;
        }
    }
    reasons
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

fn landed(rt: &Runtime, unit: UnitId, lsa: u32) -> bool {
    let lsa = lsa as usize;
    spu(rt, unit).state().ls[lsa..lsa + TRANSFER_BYTES as usize] == [MARK; TRANSFER_BYTES as usize]
}

/// The get + wait program: issue the get, request an update once every
/// masked group completes, and read the tag status into `r5`.
fn get_and_wait() -> [u32; 4] {
    [
        wrch(MFC_CMD, 2),
        wrch(MFC_WR_TAG_UPDATE, 7),
        rdch(5, MFC_RD_TAG_STAT),
        0,
    ]
}

/// [CBEA p:60 s:7.5] a get moves main-storage bytes into local storage; [CBEA p:128 s:9.3.6] its tag group reads complete once it has no outstanding operations.
#[test]
fn a_get_lands_when_it_completes_and_its_tag_reads_complete_then() {
    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, AUX_EA, LSA, TRANSFER_BYTES);
    let reasons = run(&mut rt);
    assert_eq!(
        reasons,
        [
            YieldReason::DmaSubmitted,
            YieldReason::ChannelStall,
            YieldReason::Finished
        ],
        "the wait parks until the get completes"
    );
    assert!(landed(&rt, unit, LSA), "local store holds the source bytes");
    assert_eq!(spu(&rt, unit).state().reg_word(5), 1 << TAG);
}

/// A get's source is read when it completes, so a store committed while
/// it was in flight is what lands.
#[test]
fn a_get_reads_its_source_when_it_completes() {
    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, AUX_EA, LSA, TRANSFER_BYTES);
    let step = rt.step().expect("the SPU runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the get queues");
    let source =
        ByteRange::new(GuestAddr::new(AUX_EA), u64::from(TRANSFER_BYTES)).expect("a 64-byte range");
    rt.place_bytes(
        cellgov_core::AddressSpaceId::BOOT,
        source,
        &[0x5C; TRANSFER_BYTES as usize],
    )
    .expect("the auxiliary region is writable");
    run(&mut rt);
    let lsa = LSA as usize;
    assert_eq!(
        spu(&rt, unit).state().ls[lsa..lsa + TRANSFER_BYTES as usize],
        [0x5C; TRANSFER_BYTES as usize]
    );
}

/// [CBEA p:116 s:9.1.4] zero is a valid transfer size, and a get of no bytes reads no main storage.
#[test]
fn a_zero_byte_get_completes_where_no_region_backs_its_address() {
    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, UNMAPPED_EA, LSA, 0);
    assert_eq!(run(&mut rt).last(), Some(&YieldReason::Finished));
    assert_eq!(spu(&rt, unit).state().reg_word(5), 1 << TAG);
}

/// The commit refuses a get whose source no region backs, and
/// its issuer stops rather than wait on a tag that will never complete.
#[test]
fn a_get_from_an_unmapped_address_is_refused_and_faults_the_issuer() {
    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, UNMAPPED_EA, LSA, TRANSFER_BYTES);
    let step = rt.step().expect("the SPU runs");
    let refused = rt.commit_step(&step.result, &step.effects);
    assert!(
        matches!(
            refused,
            Err(cellgov_core::CommitError::DmaSourceOutOfRange { .. })
        ),
        "{refused:?}"
    );
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Faulted)
    );
}

/// A get whose local-store end escapes the store faults at issue, and
/// one that ends at the last byte lands.
#[test]
fn a_get_at_the_end_of_local_store_lands_and_one_byte_further_faults() {
    let end = (cellgov_spu::state::SPU_LS_SIZE as u32) - TRANSFER_BYTES;
    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, AUX_EA, end, TRANSFER_BYTES);
    assert_eq!(run(&mut rt).last(), Some(&YieldReason::Finished));
    assert!(landed(&rt, unit, end));

    let (mut rt, unit) = runtime_with(&get_and_wait(), MFC_GET, AUX_EA, end + 1, TRANSFER_BYTES);
    assert_eq!(run(&mut rt), [YieldReason::Fault]);
    assert_eq!(spu(&rt, unit).status(), UnitStatus::Faulted);
}

/// A get queued before its SPU stops still lands.
#[test]
fn a_get_issued_before_a_stop_still_lands() {
    let (mut rt, unit) = runtime_with(&[wrch(MFC_CMD, 2), 0], MFC_GET, AUX_EA, LSA, TRANSFER_BYTES);
    run(&mut rt);
    rt.drain_pending_dma();
    assert!(landed(&rt, unit, LSA));
}

/// [CBE-Handbook p:528 s:19.4.3.2] the MFC SPU command queue has 16 entries; [CBEA p:113 s:9.1.1] MFC_Cmd counts the free slots, and a write to a full queue stalls until one frees.
#[test]
fn a_seventeenth_command_waits_for_a_slot() {
    let depth = MFC_SPU_QUEUE_DEPTH as usize;
    let mut program = vec![wrch(MFC_CMD, 2); depth];
    program.push(rchcnt(6, MFC_CMD));
    program.push(wrch(MFC_CMD, 2));
    program.push(0);
    let (mut rt, unit) = runtime_with(&program, MFC_PUT, 0x1000, LSA, 16);
    for _ in 0..depth {
        let step = rt.step().expect("a put issues");
        assert_eq!(step.result.yield_reason, YieldReason::DmaSubmitted);
        rt.commit_step(&step.result, &step.effects)
            .expect("the put queues");
    }
    let step = rt.step().expect("the SPU runs");
    assert_eq!(step.result.yield_reason, YieldReason::ChannelStall);
    rt.commit_step(&step.result, &step.effects)
        .expect("the park commits");
    assert_eq!(spu(&rt, unit).state().reg_word(6), 0, "no slot is free");
    assert_eq!(
        spu(&rt, unit).channel_stall().map(|stall| stall.wake),
        Some(StallWake::CommandQueueSlot)
    );
    assert_eq!(
        run(&mut rt),
        [YieldReason::DmaSubmitted, YieldReason::Finished],
        "a completion frees a slot and the write runs again"
    );
}
