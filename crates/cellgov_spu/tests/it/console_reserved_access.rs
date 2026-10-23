//! Reserved channels and the storage control commands, held to the
//! `spu_reserved_access` probe's committed capture from a raw SPU on the
//! reference console.
//!
//! An `rdch` of each of eight reserved channels reads zero and a `wrch`
//! to each has no effect; neither stops the SPU or latches a class 0
//! status. The `rchcnt` after the write is the channel's idle count. The
//! five storage control commands each complete under their tag, `sdcrz`
//! zeroes its 128-byte line, and the other four change no byte.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    self, MFC_CMD, MFC_EAH, MFC_EAL, MFC_LSA, MFC_RD_TAG_STAT, MFC_SDCRF, MFC_SDCRST, MFC_SDCRT,
    MFC_SDCRTST, MFC_SDCRZ, MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_MASK,
    MFC_WR_TAG_UPDATE, SPU_STATUS_P,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_reserved_access/ps3/cech20-cex-493/observation.json";

/// The reserved channels the probe reads and writes, in case order.
const CHANNELS: [u8; 8] = [5, 6, 10, 31, 69, 70, 71, 127];
const CASES: usize = 16;
const CASE_STRIDE: u32 = 0x20;
const SLOTS_AT: u32 = 0x2000;
const UNSTORED: u32 = 0xEEEE_EEEE;
const SL1_AT: u32 = 0x400;
const TAG_STAT_LS: u32 = 0x2200;
const BUFFER_EA_AT: u32 = 0x1F00;
const COMMANDS: [u32; 5] = [MFC_SDCRT, MFC_SDCRTST, MFC_SDCRZ, MFC_SDCRST, MFC_SDCRF];
const LINES: usize = 6;
const LINE: u32 = 0x80;
/// Where the buffer sits in CellGov's guest memory.
const BUFFER_EA: u32 = 0x1000;

/// The probe's payload, as big-endian words.
fn words() -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let w: Vec<u32> = p
        .chunks_exact(4)
        .map(|b| u32::from_be_bytes(b.try_into().expect("4 bytes")))
        .collect();
    assert_eq!((w[0], w[1]), (0, 0), "the probe ran every step");
    w
}

fn il(rt: u32, imm: u32) -> u32 {
    0x4080_0000 | (imm & 0xFFFF) << 7 | rt
}
fn a(rt: u32, ra: u32, rb: u32) -> u32 {
    0x1800_0000 | rb << 14 | ra << 7 | rt
}
fn wrch(channel: u8, rt: u32) -> u32 {
    0x21A0_0000 | u32::from(channel) << 7 | rt
}
fn rdch(rt: u32, channel: u8) -> u32 {
    0x01A0_0000 | u32::from(channel) << 7 | rt
}
fn rchcnt(rt: u32, channel: u8) -> u32 {
    0x01E0_0000 | u32::from(channel) << 7 | rt
}
fn lqa(rt: u32, addr: u32) -> u32 {
    0x3080_0000 | (addr >> 2 & 0xFFFF) << 7 | rt
}
fn stqa(rt: u32, addr: u32) -> u32 {
    0x2080_0000 | (addr >> 2 & 0xFFFF) << 7 | rt
}

/// Case `n`'s words, as `spu/main.S` places them at `0x20 n`.
fn case_words(n: usize) -> Vec<u32> {
    let channel = CHANNELS[n % 8];
    let slot = SLOTS_AT + n as u32 * 16;
    let stop = 0x2100 + n as u32;
    if n < 8 {
        vec![il(3, 0x55), rdch(3, channel), stqa(3, slot), stop]
    } else {
        vec![
            il(3, 0x66),
            wrch(channel, 3),
            rchcnt(4, channel),
            stqa(4, slot),
            stop,
        ]
    }
}

/// Runs case `n` on a fresh SPU: (SPU_Status, SPU_NPC, slot word).
fn run_case(n: usize) -> (u32, u32, u32) {
    let mut spu = SpuExecutionUnit::new(UnitId::new(0));
    let base = n * CASE_STRIDE as usize;
    for (i, w) in case_words(n).iter().enumerate() {
        spu.state_mut().ls[base + i * 4..base + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    let at = (SLOTS_AT + n as u32 * 16) as usize;
    spu.state_mut().ls[at..at + 4].copy_from_slice(&UNSTORED.to_be_bytes());
    spu.state_mut().pc = base as u32;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(Budget::new(100), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Finished, "case {n}");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let slot = u32::from_be_bytes(spu.state().ls[at..at + 4].try_into().expect("4"));
    (stop.status, stop.npc, slot)
}

/// Each reserved access runs on to its case's stop: the read stores 0,
/// and the count after the write is the idle count, 1 on the three
/// channels the console counts as 1 and 0 on the rest.
#[test]
fn a_reserved_channel_access_runs_on_as_on_the_console() {
    let w = words();
    for n in 0..CASES {
        let [status, npc, slot, class0, _] = std::array::from_fn(|i| w[4 + 5 * n + i]);
        let channel = CHANNELS[n % 8];
        assert_eq!(status, (0x2100 + n as u32) << 16 | SPU_STATUS_P, "case {n}");
        assert_eq!(class0, 0, "case {n}: no class 0 status");
        let want = if n < 8 {
            0
        } else {
            u32::from(spu::SPU_RESERVED_CHANNELS_COUNTING_ONE.contains(&channel))
        };
        assert_eq!(slot, want, "case {n}: channel {channel} on the console");
        assert_eq!(
            run_case(n),
            (status, npc, slot),
            "case {n}: channel {channel}"
        );
    }
}

/// The storage control part of `spu/main.S`, from 0x400.
fn sl1() -> Vec<u32> {
    let mut p = vec![
        lqa(11, BUFFER_EA_AT),
        il(15, u32::MAX),
        il(16, MFC_TAG_UPDATE_ALL),
        il(9, 0),
        wrch(MFC_EAH, 9),
    ];
    for (k, op) in COMMANDS.into_iter().enumerate() {
        let k = k as u32;
        p.extend([
            il(4, 0x4000 + k * LINE),
            wrch(MFC_LSA, 4),
            il(6, k * LINE),
            a(5, 11, 6),
            wrch(MFC_EAL, 5),
            il(7, 128),
            wrch(MFC_SIZE, 7),
            il(8, 1),
            wrch(MFC_TAG_ID, 8),
            il(10, op),
            wrch(MFC_CMD, 10),
            wrch(MFC_WR_TAG_MASK, 15),
            wrch(MFC_WR_TAG_UPDATE, 16),
            rdch(25, MFC_RD_TAG_STAT),
            stqa(25, TAG_STAT_LS + k * 16),
        ]);
    }
    p.push(0x2200);
    p
}

/// Word `j` of buffer line `k` before the run.
fn filled(k: u32, j: u32) -> u32 {
    0xA000_0000 | k << 8 | j
}

/// (SPU_Status, SPU_NPC, tag statuses, per line word 0 and nonzero words).
type Sl1Run = (u32, u32, [u32; 5], [[u32; 2]; LINES]);

fn run_sl1() -> Sl1Run {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(1), 20_000);
    for k in 0..LINES as u32 {
        let bytes: Vec<u8> = (0..LINE / 4)
            .flat_map(|j| filled(k, j).to_be_bytes())
            .collect();
        let at = ByteRange::new(
            GuestAddr::new(u64::from(BUFFER_EA + k * LINE)),
            u64::from(LINE),
        )
        .expect("in range");
        rt.place_bytes(AddressSpaceId::BOOT, at, &bytes)
            .expect("the buffer takes the line");
    }
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let code = SL1_AT as usize;
        for (i, w) in sl1().into_iter().enumerate() {
            state.ls[code + i * 4..code + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        let ea = BUFFER_EA_AT as usize;
        state.ls[ea..ea + 4].copy_from_slice(&BUFFER_EA.to_be_bytes());
        state.pc = SL1_AT;
        spu
    });
    let mut finished = false;
    for _ in 0..20_000 {
        let Ok(step) = rt.step() else { break };
        finished = step.result.yield_reason == YieldReason::Finished;
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
        if finished {
            break;
        }
    }
    assert!(finished, "the SPU stops");
    assert_eq!(rt.take_mfc_exception(), None);
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let ls = &spu.state().ls;
    let tags = std::array::from_fn(|k| {
        let at = (TAG_STAT_LS + k as u32 * 16) as usize;
        u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
    });
    let lines = std::array::from_fn(|k| {
        let at = ByteRange::new(
            GuestAddr::new(u64::from(BUFFER_EA + k as u32 * LINE)),
            u64::from(LINE),
        )
        .expect("in range");
        let bytes = rt.memory().read(at).expect("readable");
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| u32::from_be_bytes(b.try_into().expect("4")))
            .collect();
        [words[0], words.iter().filter(|&&w| w != 0).count() as u32]
    });
    (stop.status, stop.npc, tags, lines)
}

/// Every storage control command completes under its tag; `sdcrz`
/// zeroes its line and the other four leave theirs as they were.
#[test]
fn the_storage_control_commands_complete_and_zero_as_on_the_console() {
    let w = words();
    let tags: [u32; 5] = std::array::from_fn(|k| w[88 + k]);
    let lines: [[u32; 2]; LINES] = std::array::from_fn(|k| [w[93 + 2 * k], w[94 + 2 * k]]);
    let console = (w[84], w[85], tags, lines);
    assert_eq!(console.0, 0x2200 << 16 | SPU_STATUS_P, "the closing stop");
    assert_eq!(
        console.1,
        SL1_AT + sl1().len() as u32 * 4,
        "past the closing stop"
    );
    assert_eq!(w[86], 0, "no class 0 status");
    assert_eq!(lines[2], [0, 0], "sdcrz zeroed its line");
    for k in [0, 1, 3, 4, 5] {
        assert_eq!(lines[k], [filled(k as u32, 0), LINE / 4], "line {k} kept");
    }
    assert_eq!(run_sl1(), console);
}
