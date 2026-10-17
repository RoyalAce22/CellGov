//! Every channel instruction in the wrong direction, held to the
//! `spu_channel_direction_sweep` probe's committed capture: an rdch of
//! each of the 16 write channels and a wrch to each of the 12 read
//! channels, in one run on a raw SPU. On the console each read returns
//! zero, no write changes a nonblocking read channel, and the SPU runs on
//! to its closing stop.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_STATUS_P;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str =
    "../../tests/micro/spu_channel_direction_sweep/ps3/cech20-cex-493/observation.json";

/// The write channels the program reads, in slot order.
const WRITE_CHANNELS: [u32; 16] = [
    0x01, 0x02, 0x07, 0x09, 0x0E, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x1A, 0x1C, 0x1E,
];
/// The read channels the program writes.
const READ_CHANNELS: [u32; 12] = [
    0x00, 0x03, 0x04, 0x08, 0x0B, 0x0C, 0x0D, 0x0F, 0x18, 0x19, 0x1B, 0x1D,
];
/// The nonblocking read channels read before and after the writes.
const READ_BACK: [u32; 4] = [0x0B, 0x0C, 0x0D, 0x0F];

const SLOTS_AT: u32 = 0x1000;
const SLOTS: usize = 24;
const UNSTORED: u32 = 0xEEEE_EEEE;
const STOP_CODE: u32 = 0x2222;

/// (SPU_Status, SPU_NPC, slot words).
type Run = (u32, u32, [u32; SLOTS]);

fn console() -> Run {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    let dword = |at: usize| u64::from(word(at)) << 32 | u64::from(word(at + 4));
    assert_eq!(dword(24), 0, "no class 0 status");
    (
        word(16),
        word(20),
        std::array::from_fn(|i| word(40 + i * 4)),
    )
}

fn rdch(rt: u32, channel: u32) -> u32 {
    0x01A0_0000 | channel << 7 | rt
}

fn wrch(channel: u32, rt: u32) -> u32 {
    0x21A0_0000 | channel << 7 | rt
}

fn il(rt: u32, imm: u32) -> u32 {
    0b0_1000_0001 << 23 | (imm & 0xFFFF) << 7 | rt
}

/// `stqr rt` at `pc`, naming slot `n`.
fn stqr_slot(rt: u32, pc: u32, n: usize) -> u32 {
    let target = SLOTS_AT + n as u32 * 16;
    0b0_0100_0111 << 23 | (((target as i32 - pc as i32) >> 2) as u32 & 0xFFFF) << 7 | rt
}

/// `spu/main.S` as the SPU assembler lays it out from address 0.
fn program() -> Vec<u32> {
    let mut words = Vec::new();
    let pc = |words: &Vec<u32>| words.len() as u32 * 4;
    for (i, &ch) in READ_BACK.iter().enumerate() {
        words.push(rdch(6, ch));
        words.push(stqr_slot(6, pc(&words), 16 + i));
    }
    for (n, &ch) in WRITE_CHANNELS.iter().enumerate() {
        words.push(il(3, 0x55));
        words.push(rdch(3, ch));
        words.push(stqr_slot(3, pc(&words), n));
    }
    words.push(il(5, 0x66));
    words.extend(READ_CHANNELS.iter().map(|&ch| wrch(ch, 5)));
    for (i, &ch) in READ_BACK.iter().enumerate() {
        words.push(rdch(6, ch));
        words.push(stqr_slot(6, pc(&words), 20 + i));
    }
    words.push(STOP_CODE);
    words
}

fn run() -> Run {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    for (i, w) in program().iter().enumerate() {
        spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    let slots = SLOTS_AT as usize;
    for i in 0..SLOTS * 4 {
        spu.state_mut().ls[slots + i * 4..slots + i * 4 + 4]
            .copy_from_slice(&UNSTORED.to_be_bytes());
    }
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(
        Budget::new(1000),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(result.yield_reason, YieldReason::Finished);
    let stop = spu.stop_registers().expect("a stopped SPU");
    let ls = &spu.state().ls;
    let slot = |n: usize| {
        let at = slots + n * 16;
        u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
    };
    (stop.status, stop.npc, std::array::from_fn(slot))
}

#[test]
fn every_wrong_direction_channel_instruction_runs_on_as_on_the_console() {
    let (status, npc, slots) = console();
    assert_eq!(status, STOP_CODE << 16 | SPU_STATUS_P, "the closing stop");
    assert_eq!(npc, program().len() as u32 * 4, "past the closing stop");
    assert_eq!(slots[..16], [0; 16], "each write channel reads zero");
    assert_eq!(
        slots[16..20],
        slots[20..24],
        "no write changed a read channel"
    );

    let (cellgov_status, cellgov_npc, cellgov_slots) = run();
    assert_eq!((cellgov_status, cellgov_npc), (status, npc));
    assert_eq!(cellgov_slots[..16], slots[..16]);
    assert_eq!(cellgov_slots[16..20], cellgov_slots[20..24]);
}
