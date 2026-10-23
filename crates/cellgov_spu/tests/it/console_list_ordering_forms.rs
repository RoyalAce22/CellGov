//! The ordering forms of the `spu_list_ordering_forms` capture, run
//! through the runtime:
//!
//! - a fence list get;
//! - a list get behind a barrier command;
//! - a plain list get behind a barrier list get of its tag group;
//! - a putb behind a barrier list get.
//!
//! Each follows, with no wait, a get that brings its list into local
//! store.
//!
//! On the console each one moves what the new list names. A plain put
//! behind the barrier list get is unordered against it, and the capture
//! leaves its word volatile.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_BARRIER, MFC_CMD, MFC_EAL, MFC_GET, MFC_GETL, MFC_GETLB, MFC_GETLF, MFC_LSA, MFC_PUT,
    MFC_PUTB, MFC_RD_TAG_STAT, MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// The probe's block in CellGov's guest memory.
const BLOCK: u32 = 0x1000;
/// Where the lists sit in local store.
const LIST: u32 = 0x3800;
/// Where the first list get moves its bytes in local store.
const DST: u32 = 0x3900;
/// Where the second list get moves its bytes in local store.
const DST2: u32 = 0x3A00;
/// Where a putb writes local store back.
const OUT: u32 = 0x2000;
/// Where a plain put writes local store back.
const OUT2: u32 = 0x2100;
/// The first register a command's five channel operands occupy.
const FIRST_REG: u32 = 20;

/// The capture's result words, in this order:
///
/// 1. the getlf;
/// 2. the getl behind the barrier command;
/// 3. the getlb;
/// 4. the getl after it;
/// 5. the putb;
/// 6. the plain put.
fn console() -> [u32; 6] {
    let path = "../../tests/micro/spu_list_ordering_forms/ps3/cech20-cex-493/observation.json";
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 64);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "no step of the probe failed");
    assert_eq!(word(20), 0, "the SPU exited cleanly");
    std::array::from_fn(|i| word(32 + 4 * i))
}

/// `wrch channel, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// One MFC command: local store address, effective address, size, tag
/// and opcode.
#[derive(Clone, Copy)]
struct Command {
    lsa: u32,
    ea: u32,
    size: u32,
    tag: u32,
    opcode: u32,
}

/// A get of 16 bytes of the block, at `element`, over the lists.
fn get_lists(element: u32, tag: u32) -> Command {
    Command {
        lsa: LIST,
        ea: BLOCK + element,
        size: 16,
        tag,
        opcode: MFC_GET,
    }
}

/// A one-element list command whose list sits at `list`.
fn list(opcode: u32, dst: u32, list: u32, tag: u32) -> Command {
    Command {
        lsa: dst,
        ea: list,
        size: 8,
        tag,
        opcode,
    }
}

/// What a run leaves behind.
struct Outcome {
    dst: u32,
    dst2: u32,
    out: u32,
    out2: u32,
    held: usize,
}

/// Runs `commands` back to back on one SPU, then waits for every tag
/// group. The block holds each `(element, fresh)` pair: a list element at
/// `BLOCK + element` naming 16 bytes of `fresh`. Local store's lists name
/// `stale` bytes; [`DST`] starts as 0x6C bytes.
fn run(commands: &[Command], elements: &[(u32, u8)], stale: [u8; 2]) -> Outcome {
    let mut memory = GuestMemory::new(0x8000);
    let mut put = |at: u32, bytes: &[u8]| {
        let range = cellgov_mem::ByteRange::new(GuestAddr::new(u64::from(at)), bytes.len() as u64)
            .expect("in range");
        memory.apply_commit(range, bytes).expect("in memory");
    };
    for (i, &(element, fresh)) in elements.iter().enumerate() {
        let fresh_at = 0x100 + 0x10 * i as u32;
        put(BLOCK + element, &16u32.to_be_bytes());
        put(BLOCK + element + 4, &(BLOCK + fresh_at).to_be_bytes());
        put(BLOCK + fresh_at, &[fresh; 16]);
    }
    put(BLOCK + 0x200, &[stale[0]; 16]);
    put(BLOCK + 0x210, &[stale[1]; 16]);
    let tags = commands.iter().fold(0, |m, c| m | 1 << c.tag);
    let mut rt = Runtime::new(memory, Budget::new(100), 400);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let mut program = Vec::new();
        for (i, c) in commands.iter().enumerate() {
            let r = FIRST_REG + 5 * i as u32;
            program.extend([
                wrch(MFC_LSA, r),
                wrch(MFC_EAL, r + 1),
                wrch(MFC_SIZE, r + 2),
                wrch(MFC_TAG_ID, r + 3),
                wrch(MFC_CMD, r + 4),
            ]);
            for (k, value) in [c.lsa, c.ea, c.size, c.tag, c.opcode]
                .into_iter()
                .enumerate()
            {
                state.set_reg_word_splat((r + k as u32) as u8, value);
            }
        }
        program.extend([
            wrch(MFC_WR_TAG_UPDATE, 7),
            (0x00D << 21) | (u32::from(MFC_RD_TAG_STAT) << 7) | 5,
            0,
        ]);
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        for (i, eal) in [BLOCK + 0x200, BLOCK + 0x210].into_iter().enumerate() {
            let at = LIST as usize + 8 * i;
            state.ls[at..at + 4].copy_from_slice(&16u32.to_be_bytes());
            state.ls[at + 4..at + 8].copy_from_slice(&eal.to_be_bytes());
        }
        let dst = DST as usize;
        state.ls[dst..dst + 16].copy_from_slice(&[0x6C; 16]);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.channels.tag_mask = tags;
        spu
    });
    // The bound is well past the probe's few steps, so a run that never
    // reaches the stop ends here and does not spin.
    let mut reasons = Vec::new();
    let mut held = 0;
    for _ in 0..400 {
        let Ok(mut step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        // A held list's issue queues nothing; every other MFC command
        // queues a transfer or a refusal.
        if step.result.yield_reason == YieldReason::DmaSubmitted && step.effects.is_empty() {
            held += 1;
        }
        rt.commit_step(&mut step).expect("the step commits");
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
    assert_eq!(state.reg_word(5), tags, "every tag group completed");
    let ls = |at: u32| {
        let at = at as usize;
        u32::from_be_bytes(state.ls[at..at + 4].try_into().expect("4 bytes"))
    };
    let main = |at: u32| {
        let range =
            cellgov_mem::ByteRange::new(GuestAddr::new(u64::from(at)), 4).expect("in range");
        u32::from_be_bytes(
            rt.memory()
                .read(range)
                .expect("main memory")
                .try_into()
                .expect("4"),
        )
    };
    Outcome {
        dst: ls(DST),
        dst2: ls(DST2),
        out: main(OUT),
        out2: main(OUT2),
        held,
    }
}

/// [CBEA p:69 s:7.9] a fence orders the command after the preceding commands of its tag group.
#[test]
fn a_fence_list_reads_the_list_its_tag_group_brought_in_as_on_the_console() {
    let [fence, ..] = console();
    let got = run(
        &[get_lists(0x00, 3), list(MFC_GETLF, DST, LIST, 3)],
        &[(0x00, 0xA1)],
        [0x1A, 0x1A],
    );
    assert_eq!((got.dst, got.held), (fence, 1));
}

/// [CBEA p:72 s:7.9.3] a barrier command orders every later command after every earlier one, whatever their tag groups.
#[test]
fn a_list_behind_a_barrier_command_reads_the_list_another_tag_brought_in_as_on_the_console() {
    let [_, behind_barrier, ..] = console();
    let barrier = Command {
        lsa: 0,
        ea: 0,
        size: 0,
        tag: 4,
        opcode: MFC_BARRIER,
    };
    let got = run(
        &[get_lists(0x10, 4), barrier, list(MFC_GETL, DST, LIST, 5)],
        &[(0x10, 0xB2)],
        [0x2B, 0x2B],
    );
    assert_eq!((got.dst, got.held), (behind_barrier, 1));
}

/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
#[test]
fn a_plain_list_behind_a_barrier_list_of_its_group_reads_the_new_list_as_on_the_console() {
    let [_, _, first, second, ..] = console();
    let got = run(
        &[
            get_lists(0x20, 7),
            list(MFC_GETLB, DST, LIST, 7),
            list(MFC_GETL, DST2, LIST + 8, 7),
        ],
        &[(0x20, 0xD1), (0x28, 0xD2)],
        [0x3D, 0x4D],
    );
    assert_eq!((got.dst, got.dst2, got.held), (first, second, 2));
}

/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
///
/// The putb's barrier orders it behind the getlb. The getlb's barrier
/// orders the plain put only behind the get, so either the gathered bytes or the buffer before the
/// gather is a legal answer for it.
#[test]
fn a_putb_behind_a_barrier_list_writes_what_the_list_brought_in_as_on_the_console() {
    let [.., putb, plain] = console();
    let got = run(
        &[
            get_lists(0x30, 6),
            list(MFC_GETLB, DST, LIST, 6),
            Command {
                lsa: DST,
                ea: OUT2,
                size: 16,
                tag: 6,
                opcode: MFC_PUT,
            },
            Command {
                lsa: DST,
                ea: OUT,
                size: 16,
                tag: 6,
                opcode: MFC_PUTB,
            },
        ],
        &[(0x30, 0xC3)],
        [0x5C, 0x5C],
    );
    assert_eq!((got.out, got.dst, got.held), (putb, putb, 1));
    assert!(
        [putb, 0x6C6C_6C6C].contains(&plain),
        "the console's plain put read {plain:#010x}"
    );
    assert!(
        [putb, 0x6C6C_6C6C].contains(&got.out2),
        "CellGov's plain put read {:#010x}",
        got.out2
    );
}
