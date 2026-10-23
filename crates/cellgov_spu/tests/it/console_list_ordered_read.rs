//! A list get issued straight after a get of its own list, in one tag
//! group, as the `spu_list_ordered_read` capture shows it, run through
//! the runtime.
//!
//! On the console the getlb moves what the new list names: its barrier
//! holds it until the get lands. The getl's read is unordered against the
//! get, and the capture leaves its word volatile. CellGov holds the getlb
//! and reads the getl's list at issue.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAL, MFC_GET, MFC_GETL, MFC_GETLB, MFC_LSA, MFC_PUT, MFC_RD_TAG_STAT, MFC_SIZE,
    MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// The probe's block in CellGov's guest memory.
const BLOCK: u32 = 0x1000;
/// Where the list sits in local store.
const LIST: u32 = 0x3800;
/// Where the list get moves its bytes in local store.
const DST: u32 = 0x3900;
/// The tag group of both commands.
const TAG: u32 = 1;

/// The capture's first words of the getlb and the getl.
fn console() -> (u32, u32) {
    let path = "../../tests/micro/spu_list_ordered_read/ps3/cech20-cex-493/observation.json";
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 48);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "no step of the probe failed");
    assert_eq!(word(20), 0, "the SPU exited cleanly");
    (word(32), word(36))
}

/// `wrch channel, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// Where [`run`]'s put, when it has one, writes the list's bytes back.
const OUT: u32 = 0x2000;

/// Runs one case of the probe on one SPU.
///
/// - Local store's list names the 16 bytes at `BLOCK + stale`.
/// - The get brings in the block's element at `BLOCK + element`, which
///   names `BLOCK + fresh`.
/// - `list_cmd` reads the list.
///
/// Returns the first word the list get moved, and whether the runtime
/// held the list get at its issue.
fn run(list_cmd: u32, element: u32, fresh: u32, stale: u32) -> (u32, bool) {
    let (moved, _, held) = run_with_put(list_cmd, element, fresh, stale, false);
    (moved, held)
}

/// [`run`], and with `put_back` a plain put of the list's destination to
/// [`OUT`] issued straight after the list get, in its tag group.
///
/// Returns the first word the list get moved, the first word at [`OUT`],
/// and whether the runtime held the list get at its issue.
fn run_with_put(
    list_cmd: u32,
    element: u32,
    fresh: u32,
    stale: u32,
    put_back: bool,
) -> (u32, u32, bool) {
    let mut memory = GuestMemory::new(0x8000);
    let mut put = |at: u32, bytes: &[u8]| {
        let range = cellgov_mem::ByteRange::new(GuestAddr::new(u64::from(at)), bytes.len() as u64)
            .expect("in range");
        memory.apply_commit(range, bytes).expect("in memory");
    };
    put(BLOCK + element, &16u32.to_be_bytes());
    put(BLOCK + element + 4, &(BLOCK + fresh).to_be_bytes());
    put(BLOCK + fresh, &[0xF1; 16]);
    put(BLOCK + stale, &[0x5A; 16]);
    let mut rt = Runtime::new(memory, Budget::new(100), 400);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let mut program = vec![
            // The get of the list.
            wrch(MFC_LSA, 11),
            wrch(MFC_EAL, 12),
            wrch(MFC_SIZE, 13),
            wrch(MFC_TAG_ID, 14),
            wrch(MFC_CMD, 10),
            // The list get, in the same tag group, with no wait between.
            wrch(MFC_LSA, 15),
            wrch(MFC_EAL, 11),
            wrch(MFC_SIZE, 16),
            wrch(MFC_CMD, 17),
        ];
        if put_back {
            // The put of the list's destination, again with no wait.
            program.extend([wrch(MFC_EAL, 18), wrch(MFC_SIZE, 13), wrch(MFC_CMD, 19)]);
        }
        program.extend([
            wrch(MFC_WR_TAG_UPDATE, 7),
            (0x00D << 21) | (u32::from(MFC_RD_TAG_STAT) << 7) | 5,
            0,
        ]);
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let list = LIST as usize;
        state.ls[list..list + 4].copy_from_slice(&16u32.to_be_bytes());
        state.ls[list + 4..list + 8].copy_from_slice(&(BLOCK + stale).to_be_bytes());
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.set_reg_word_splat(10, MFC_GET);
        state.set_reg_word_splat(11, LIST);
        state.set_reg_word_splat(12, BLOCK + element);
        state.set_reg_word_splat(13, 16);
        state.set_reg_word_splat(14, TAG);
        state.set_reg_word_splat(15, DST);
        state.set_reg_word_splat(16, 8);
        state.set_reg_word_splat(17, list_cmd);
        state.set_reg_word_splat(18, OUT);
        state.set_reg_word_splat(19, MFC_PUT);
        state.channels.tag_mask = 1 << TAG;
        spu
    });
    // The bound is well past the probe's few steps, so a run that never
    // reaches the stop ends here and does not spin.
    let mut reasons = Vec::new();
    let mut held = false;
    for _ in 0..400 {
        let Ok(step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        // A held list's issue queues nothing; every other MFC command
        // queues a transfer or a refusal. A get that lands before the
        // list issues would leave the result right for the wrong reason.
        held |= step.result.yield_reason == YieldReason::DmaSubmitted && step.effects.is_empty();
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
        if step.result.yield_reason == YieldReason::Finished {
            break;
        }
    }
    assert_eq!(reasons.last(), Some(&YieldReason::Finished), "{reasons:?}");
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    let state = spu.state();
    assert_eq!(state.reg_word(5), 1 << TAG, "the tag group completed");
    let dst = DST as usize;
    let moved = u32::from_be_bytes(state.ls[dst..dst + 4].try_into().expect("4 bytes"));
    let out = rt
        .memory()
        .read(cellgov_mem::ByteRange::new(GuestAddr::new(u64::from(OUT)), 4).expect("in range"))
        .expect("the out buffer");
    let out = u32::from_be_bytes(out.try_into().expect("4 bytes"));
    (moved, out, held)
}

/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
#[test]
fn a_barrier_list_reads_the_list_its_tag_group_brought_in_as_on_the_console() {
    let (barrier, _) = console();
    assert_eq!(barrier, 0xF1F1_F1F1, "the console read the new list");
    assert_eq!(run(MFC_GETLB, 0x00, 0x80, 0xC0), (barrier, true));
}

/// [CBEA p:53 s:7.1.1] unless a form says otherwise, data-transfer commands execute in any order.
#[test]
fn a_plain_list_reads_the_list_local_store_held_at_issue_as_on_the_console() {
    let (_, plain) = console();
    // The capture leaves this word volatile: the old list (0x4B) and the
    // new one (0xE2) are both legal reads.
    assert!(
        [0x4B4B_4B4B, 0xE2E2_E2E2].contains(&plain),
        "the console's getl read {plain:#010x}"
    );
    // The capture's 0x4B bytes stand where this case puts 0x5A.
    assert_eq!(run(MFC_GETL, 0x10, 0x90, 0xD0), (0x5A5A_5A5A, false));
}

/// The getlb's barrier orders a later plain put of its group behind the
/// get only, so the put may read its buffer before or after the list
/// lands. CellGov stalls the put until the held list starts. That is one
/// of the legal orders, and the put then writes back the bytes the list
/// brought in.
///
/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
#[test]
fn a_put_behind_a_held_barrier_list_writes_what_the_list_brought_in() {
    assert_eq!(
        run_with_put(MFC_GETLB, 0x00, 0x80, 0xC0, true),
        (0xF1F1_F1F1, 0xF1F1_F1F1, true)
    );
}
