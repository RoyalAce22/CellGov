//! A queued transfer reads and writes local store when it completes, so
//! what the issuer does to its buffer while the transfer is in flight is
//! what the transfer moves.

// [CBEA p:173 s:10.3] the local-storage access of a queued command is complete when its tag group reads complete; a tag-specific fence orders the local-storage accesses of the earlier commands of its group before its own.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_BARRIER, MFC_CMD, MFC_EAL, MFC_EIEIO, MFC_GET, MFC_LSA, MFC_PUT, MFC_PUTB, MFC_PUTF,
    MFC_RD_TAG_STAT, MFC_SIZE, MFC_SYNC, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_MASK,
    MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 4;
/// The local-store buffer both tests move.
const BUFFER: u32 = 0x400;

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `stqd rt, 0(ra)`.
fn stqd(rt: u32, ra: u32) -> u32 {
    (0x24 << 24) | (ra << 7) | rt
}

/// The tail every program ends with: wait for the tag group, then stop.
fn wait_and_stop() -> [u32; 4] {
    [
        wrch(MFC_WR_TAG_MASK, 19),
        wrch(MFC_WR_TAG_UPDATE, 20),
        rdch(MFC_RD_TAG_STAT, 21),
        0,
    ]
}

/// Run one SPU with `program` and `regs` over `memory` until it stops,
/// then return the memory and the unit's local-store buffer.
fn run(memory: GuestMemory, program: &[u32], regs: &[(u8, u32)]) -> (Runtime, Vec<u8>) {
    let mut rt = Runtime::new(memory, Budget::new(1), 200);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().chain(&wait_and_stop()).enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let buffer = BUFFER as usize;
        state.ls[buffer..buffer + 16].copy_from_slice(&[0xA5; 16]);
        for &(reg, value) in
            regs.iter()
                .chain(&[(13, TAG), (19, 1 << TAG), (20, MFC_TAG_UPDATE_ALL)])
        {
            state.set_reg_word_splat(reg, value);
        }
        spu
    });
    let mut reasons = Vec::new();
    for _ in 0..200 {
        let Ok(step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if step.result.yield_reason == YieldReason::Finished {
            break;
        }
    }
    assert_eq!(reasons.last(), Some(&YieldReason::Finished), "{reasons:?}");
    assert_eq!(rt.take_mfc_exception(), None);
    let buffer = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit")
        .state()
        .ls[BUFFER as usize..BUFFER as usize + 16]
        .to_vec();
    (rt, buffer)
}

fn read(rt: &Runtime, ea: u64) -> Vec<u8> {
    rt.memory()
        .read(ByteRange::new(GuestAddr::new(ea), 16).expect("in range"))
        .expect("readable")
        .to_vec()
}

#[test]
fn a_put_carries_a_store_the_issuer_made_while_it_was_in_flight() {
    let program = [
        wrch(MFC_LSA, 10),
        wrch(MFC_EAL, 11),
        wrch(MFC_SIZE, 12),
        wrch(MFC_TAG_ID, 13),
        wrch(MFC_CMD, 14),
        stqd(22, 10),
    ];
    let regs = [
        (10, BUFFER),
        (11, 0x100),
        (12, 16),
        (14, MFC_PUT),
        (22, 0x5A5A_5A5A),
    ];
    let (rt, _) = run(GuestMemory::new(0x1000), &program, &regs);
    assert_eq!(read(&rt, 0x100), [0x5A; 16]);
}

/// A get of [`BUFFER`] from 0x200, then `between` if any, then `put` of
/// [`BUFFER`] to 0x300, all under one tag and with no wait between them.
/// Returns the buffer and the bytes at 0x300 once the tag group reads
/// complete.
fn get_then_put(between: Option<u32>, put: u32) -> (Vec<u8>, Vec<u8>) {
    let mut memory = GuestMemory::new(0x1000);
    let source = ByteRange::new(GuestAddr::new(0x200), 16).expect("in range");
    memory.apply_commit(source, &[0xC3; 16]).expect("writable");
    let mut program = vec![
        wrch(MFC_LSA, 10),
        wrch(MFC_EAL, 11),
        wrch(MFC_SIZE, 12),
        wrch(MFC_TAG_ID, 13),
        wrch(MFC_CMD, 14),
    ];
    if between.is_some() {
        program.extend([wrch(MFC_TAG_ID, 13), wrch(MFC_CMD, 17)]);
    }
    program.extend([
        wrch(MFC_LSA, 10),
        wrch(MFC_EAL, 15),
        wrch(MFC_SIZE, 12),
        wrch(MFC_TAG_ID, 13),
        wrch(MFC_CMD, 16),
    ]);
    let regs = [
        (10, BUFFER),
        (11, 0x200),
        (12, 16),
        (14, MFC_GET),
        (15, 0x300),
        (16, put),
        (17, between.unwrap_or(0)),
    ];
    let (rt, buffer) = run(memory, &program, &regs);
    (buffer, read(&rt, 0x300))
}

#[test]
fn an_ordered_put_after_a_get_of_its_buffer_carries_the_bytes_the_get_landed() {
    let wrong: Vec<_> = [
        ("putf", None, MFC_PUTF),
        ("putb", None, MFC_PUTB),
        ("barrier then put", Some(MFC_BARRIER), MFC_PUT),
        ("mfcsync then put", Some(MFC_SYNC), MFC_PUT),
        ("mfceieio then put", Some(MFC_EIEIO), MFC_PUT),
    ]
    .into_iter()
    .map(|(form, between, put)| (form, get_then_put(between, put)))
    .filter(|(_, (buffer, put))| *buffer != [0xC3; 16] || *put != [0xC3; 16])
    .collect();
    assert!(wrong.is_empty(), "(form, (buffer, put)): {wrong:x?}");
}
