//! The SPU's instruction decode, held to the `spu_opcode_sweep` probe's
//! committed capture: each of the 2048 11-bit opcode prefixes, with every
//! other field zero, run once on a raw SPU and followed by a stop.
//!
//! Every row of `SPU_OPCODE_MAP` the CBE provides decodes on the console,
//! and the five it does not stop the SPU as invalid instructions. CellGov
//! ends every block as the console does, apart from four named
//! differences.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    SPU_LS_SIZE, SPU_STATUS_H, SPU_STATUS_I, SPU_STATUS_P, SPU_STATUS_W,
};
use cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP;
use cellgov_ps3_abi::lv2::spu::raw_spu::INT_STAT_SPU_ERROR;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_opcode_sweep/ps3/cech20-cex-493/observation.json";

const PREFIXES: usize = 2048;
const BLOCKS_AT: u32 = 0x1000;
const BLOCK: u32 = 16;
/// `il $0, 0`.
const IL_R0_0: u32 = 0x4080_0000;
/// What the PPU fills the local store with before it loads the image.
const FILL: u32 = 0x0000_3FF0;
const BLOCK_TAIL: u32 = 0x0000_3FF1;

/// The unassigned prefixes the console runs on from, where CellGov stops
/// on each as an invalid instruction. What they do with nonzero fields
/// is not known.
const UNASSIGNED_THE_CONSOLE_RUNS: [std::ops::RangeInclusive<u32>; 13] = [
    0x008..=0x00B,
    0x010..=0x01F,
    0x114..=0x117,
    0x12C..=0x12F,
    0x145..=0x147,
    0x154..=0x157,
    0x1AD..=0x1AF,
    0x1C0..=0x1C3,
    0x1D0..=0x1D3,
    0x1E8..=0x1EF,
    0x200..=0x200,
    0x202..=0x203,
    0x378..=0x37F,
];

/// The prefix whose `stqr` zeroes its own block, `stop` included. The
/// console runs the stop it already fetched; CellGov, which models no
/// instruction fetch ahead of a store, fetches the zeroed word and stops
/// with code 0.
const STQR_OVER_ITS_BLOCK: u32 = 0x11C;

/// The prefix whose `rdch $0, $ch0` waits until the PPU stops it. The
/// console's SPU_Status then holds no W; CellGov sets W, as the CBEA
/// describes.
const WAITS_ON_CHANNEL_0: u32 = 0x00D;

/// Whether block `p`'s word is a halt whose condition holds. Every
/// register operand is `$0`, which the block's `il` zeroes. An immediate
/// form's I10 takes the prefix's low three bits as its top bits.
fn halts(p: u32) -> bool {
    let imm = (((p & 7) << 7) as i32) << 22 >> 22;
    SPU_OPCODE_MAP
        .iter()
        .filter(|row| row.matches(p << 21))
        .any(|row| match row.mnemonic {
            // [SPU-ISA p:150 s:7] heq halts when RA's preferred word equals RB's.
            "heq" => true,
            // [SPU-ISA p:151 s:7] heqi compares against I10 sign-extended to 32 bits.
            "heqi" => imm == 0,
            // [SPU-ISA p:153 s:7] hgti compares algebraically against the sign-extended I10.
            "hgti" => 0 > imm,
            // hgt and hlgt compare 0 with 0, and hlgti compares 0 unsigned:
            // none is met.
            _ => false,
        })
}

/// (SPU_Status, SPU_NPC, class 0 status low word) per prefix.
fn console() -> Vec<[u32; 3]> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let w: Vec<u32> = p
        .chunks_exact(4)
        .map(|b| u32::from_be_bytes(b.try_into().expect("4 bytes")))
        .collect();
    assert_eq!((w[0], w[1]), (0, 0), "the probe ran every step");
    (0..PREFIXES)
        .map(|p| [w[4 + 3 * p], w[5 + 3 * p], w[6 + 3 * p]])
        .collect()
}

fn block_at(p: u32) -> u32 {
    BLOCKS_AT + p * BLOCK
}

/// Whether block `p` ran on to its own stop on the console.
fn ran(record: [u32; 3], p: u32) -> bool {
    record[0] == (0x2000 + p) << 16 | SPU_STATUS_P && record[1] == block_at(p) + 12
}

/// Whether block `p` stopped on its word as an invalid instruction.
fn invalid(record: [u32; 3]) -> bool {
    record[0] == SPU_STATUS_I && u64::from(record[2]) == INT_STAT_SPU_ERROR
}

fn unassigned_the_console_runs(p: u32) -> bool {
    UNASSIGNED_THE_CONSOLE_RUNS.iter().any(|r| r.contains(&p))
}

/// The local store the probe runs in: the fill, then the image, which is
/// zero below the blocks.
fn local_store() -> Vec<u8> {
    let mut ls: Vec<u8> = (0..SPU_LS_SIZE / 4)
        .flat_map(|_| FILL.to_be_bytes())
        .collect();
    ls[..BLOCKS_AT as usize].fill(0);
    for p in 0..PREFIXES as u32 {
        let at = block_at(p) as usize;
        for (i, w) in [IL_R0_0, p << 21, 0x2000 + p, BLOCK_TAIL]
            .iter()
            .enumerate()
        {
            ls[at + i * 4..at + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
    }
    ls
}

/// Every block run in order on one SPU, as the PPU runs them: write
/// SPU_NPC, run, and stop the SPU from outside if it is still running or
/// waiting on a channel when the budget ends. (SPU_Status, SPU_NPC) per
/// block.
fn cellgov() -> Vec<(u32, u32)> {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    spu.state_mut().ls.copy_from_slice(&local_store());
    spu.request_stop(false).expect("problem state");
    let mem = GuestMemory::new(0x1000);
    (0..PREFIXES as u32)
        .map(|p| {
            spu.write_npc(block_at(p)).expect("a stopped SPU");
            spu.restart().expect("the SPU restarts");
            let mut effects = Vec::new();
            let result =
                spu.run_until_yield(Budget::new(64), &ExecutionContext::new(&mem), &mut effects);
            if result.yield_reason != YieldReason::Finished {
                let waiting = result.yield_reason == YieldReason::ChannelStall;
                spu.request_stop(waiting).expect("problem state");
            }
            let stop = spu.stop_registers().expect("a stopped SPU");
            (stop.status, stop.npc)
        })
        .collect()
}

#[test]
fn every_opcode_map_row_decodes_on_the_console_exactly_when_it_is_on_the_cbe() {
    let console = console();
    let mut rows_seen = 0;
    for row in SPU_OPCODE_MAP {
        let prefixes: Vec<u32> = (0..PREFIXES as u32)
            .filter(|&p| row.matches(p << 21))
            .collect();
        assert_eq!(prefixes.len(), 1 << (11 - row.width), "{}", row.mnemonic);
        for p in prefixes {
            assert_eq!(
                invalid(console[p as usize]),
                !row.on_cbe,
                "{} at prefix {p:#05x}",
                row.mnemonic
            );
        }
        rows_seen += 1;
    }
    assert_eq!(rows_seen, SPU_OPCODE_MAP.len());
}

#[test]
fn an_unassigned_prefix_stops_as_invalid_unless_the_console_runs_it() {
    let console = console();
    let unassigned: Vec<u32> = (0..PREFIXES as u32)
        .filter(|&p| !SPU_OPCODE_MAP.iter().any(|row| row.matches(p << 21)))
        .collect();
    assert!(!unassigned.is_empty());
    for p in unassigned {
        let record = console[p as usize];
        if unassigned_the_console_runs(p) {
            assert!(ran(record, p), "{p:#05x} runs on the console");
        } else {
            assert!(invalid(record), "{p:#05x} stops with I on the console");
        }
    }
}

/// Every block ends on CellGov as it did on the console, apart from four
/// named differences:
///
/// - the unassigned prefixes the console runs;
/// - a met halt, after which the console runs on into the block's stop
///   while CellGov halts at once;
/// - the `stqr` over its own block;
/// - the stop of an SPU waiting on channel 0.
///
/// An invalid word resumes 4 bytes past it on both: it sits in a block's
/// odd word slot, so it cannot issue as the first of a pair.
#[test]
fn every_block_ends_as_on_the_console_apart_from_four_named_differences() {
    let console = console();
    let runs = cellgov();
    assert_eq!((0..PREFIXES as u32).filter(|&p| halts(p)).count(), 6);
    for p in 0..PREFIXES as u32 {
        let record = console[p as usize];
        let (status, npc) = runs[p as usize];
        let at = block_at(p);
        if invalid(record) {
            assert_eq!(record[1], at + 8, "{p:#05x}: past the word");
            assert_eq!((status, npc), (record[0], record[1]), "{p:#05x}");
        } else if unassigned_the_console_runs(p) {
            assert_eq!((status, npc), (SPU_STATUS_I, at + 8), "{p:#05x}");
        } else if halts(p) {
            assert_eq!(status, SPU_STATUS_H, "{p:#05x}: CellGov halts");
            assert!(ran(record, p), "{p:#05x}: the console ran into the stop");
            assert_eq!(npc, at + 8, "{p:#05x}: CellGov halts at once");
        } else if p == STQR_OVER_ITS_BLOCK {
            assert!(ran(record, p), "{p:#05x}: the console ran the fetched stop");
            assert_eq!((status, npc), (SPU_STATUS_P, at + 12), "{p:#05x}");
        } else if p == WAITS_ON_CHANNEL_0 {
            assert_eq!(record[..2], [0, at + 4], "{p:#05x}: no W on the console");
            assert_eq!((status, npc), (SPU_STATUS_W, at + 4), "{p:#05x}");
        } else {
            assert_eq!((status, npc), (record[0], record[1]), "{p:#05x}");
        }
    }
}
