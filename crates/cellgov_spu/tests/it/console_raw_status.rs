//! A Raw SPU run to `stop 0x1234` through its problem-state window, held
//! to the `spu_raw_status` probe's committed capture: `SPU_Status`,
//! `SPU_NPC`, and the class 0, 1 and 2 interrupt status the PPU reads
//! back after the stop.

use cellgov_ps3_abi::hw::spu::{SPU_NPC_OFFSET, SPU_RUN_CNTL_OFFSET, SPU_STATUS_OFFSET};
use cellgov_ps3_abi::lv2::spu::raw_spu;
use cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_INT_STAT;

use crate::raw_spu_window::{
    create_ops, poll_stopped, read, register, run_ops, runtime, Op, OUT_PTR, WINDOW,
};

const CONSOLE: &str = "../../tests/micro/spu_raw_status/ps3/cech20-cex-493/observation.json";

/// `tests/micro/spu_raw_status/spu/main.S` as the SPU assembler encodes
/// it (`spu-objdump` of the built image): `il $3,0x55`, `nop $127`,
/// `stop 0x1234`, `stop 0x2222`.
const PROGRAM: [u32; 4] = [0x4080_2a83, 0x4020_007f, 0x0000_1234, 0x0000_2222];

/// The interrupt classes the probe reads, in its order.
const CLASSES: [u32; 3] = [raw_spu::CLASS_ERROR, 1, raw_spu::CLASS_APPLICATION];

/// What the console reported: `SPU_Status`, `SPU_NPC`, and the status
/// word of each class in [`CLASSES`].
fn console() -> (u32, u32, [u64; 3]) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    let dword = |at: usize| u64::from(word(at)) << 32 | u64::from(word(at + 4));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (word(8), word(12), [dword(40), dword(48), dword(56)])
}

/// The class 2 status holds the stop-and-signal S beside the mailbox
/// threshold B, class 0 holds nothing, and the refused class 1 read
/// leaves its zeroed word alone.
#[test]
fn a_stopped_raw_spu_reports_the_consoles_status_and_interrupts() {
    let (status, npc, stats) = console();
    let mut ops = create_ops();
    ops.extend(
        PROGRAM
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + 4 * i as u64, w)),
    );
    ops.extend([
        Op::Store(register(SPU_NPC_OFFSET), 0),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
        poll_stopped(),
    ]);
    ops.extend(CLASSES.iter().enumerate().map(|(k, &class)| {
        Op::Call([
            RAW_SPU_GET_INT_STAT,
            0,
            u64::from(class),
            OUT_PTR + 8 * k as u64,
        ])
    }));
    let n = ops.len();
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, n);
    let dword = |at: u64| u64::from(read(&rt, at)) << 32 | u64::from(read(&rt, at + 4));
    assert_eq!(read(&rt, register(SPU_STATUS_OFFSET)), status);
    assert_eq!(read(&rt, register(SPU_NPC_OFFSET)), npc);
    for (k, &console) in stats.iter().enumerate() {
        assert_eq!(
            dword(OUT_PTR + 8 * k as u64),
            console,
            "class {}",
            CLASSES[k]
        );
    }
}
