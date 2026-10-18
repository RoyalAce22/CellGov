//! The SPU events and the decrementer as a retail console shows them:
//! the `spu_events` probe's committed capture.
//!
//! The capture pins what the console does: every event the probe arms
//! fires once its condition holds, a pending event latches while masked
//! and counts the moment software enables it, the decrementer runs at the
//! time-base frequency, and acknowledging Tm with the event masked stops
//! it. CellGov's SPU runs the phases it can run alone (Ms, Tm, the
//! masked latch and the decrementer's stop) and must read back the same
//! words.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::state::SpuState;
use cellgov_time::GuestTicks;

const CONSOLE: &str = "../../tests/micro/spu_events/ps3/cech20-cex-493/observation.json";

/// The probe's poll bound: a count that never rose reads this.
const MAX_POLLS: u32 = 1_000_000;

/// The probe's 224-byte payload: twelve 16-byte phase records, then the
/// time base at the two rate flags, its frequency, the missed flag and
/// the failed step.
struct Capture(Vec<u8>);

impl Capture {
    fn load() -> Self {
        let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
            .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
        let payload = observation.memory_regions[0].data.clone();
        assert_eq!(payload.len(), 224);
        let capture = Self(payload);
        assert_eq!(
            (capture.word(216), capture.word(220)),
            (0, 0),
            "every flag came and no step failed"
        );
        capture
    }

    fn word(&self, at: usize) -> u32 {
        u32::from_be_bytes(self.0[at..at + 4].try_into().expect("4 bytes"))
    }

    fn doubleword(&self, at: usize) -> u64 {
        u64::from_be_bytes(self.0[at..at + 8].try_into().expect("8 bytes"))
    }

    /// Phase `phase`'s four words.
    fn record(&self, phase: usize) -> [u32; 4] {
        [0, 4, 8, 12].map(|at| self.word(16 * phase + at))
    }
}

#[test]
fn every_probed_event_fires_on_the_console_once_its_condition_holds() {
    let capture = Capture::load();
    for (phase, event) in [
        (1, spu::event::MS),
        (2, spu::event::TG),
        (3, spu::event::QV),
        (4, spu::event::SN),
        (5, spu::event::TM),
        (6, spu::event::MB),
        (7, spu::event::S1),
        (8, spu::event::LR),
    ] {
        let [status, polls, count_after, _] = capture.record(phase);
        assert_eq!(status, event, "phase {phase} reads its own event");
        assert!(polls < MAX_POLLS, "phase {phase} fired");
        assert_eq!(
            count_after, 0,
            "phase {phase}: acknowledged, the event does not count again"
        );
    }
    assert_eq!(capture.record(3)[3], 0, "the QV phase filled the queue");
    assert_eq!(
        capture.record(4)[3],
        1 << 3,
        "the SN phase's list stall status names its tag group"
    );
}

#[test]
fn the_decrementer_runs_at_the_time_base_and_stops_on_a_masked_tm_acknowledgment() {
    let capture = Capture::load();
    let [dec_a, dec_b, spin, _] = capture.record(11);
    assert_eq!(spin, 1 << 28);
    let dec_ticks = u64::from(dec_a.wrapping_sub(dec_b));
    let tb_ticks = capture.doubleword(200) - capture.doubleword(192);
    assert_eq!(
        capture.doubleword(208),
        79_800_000,
        "the time-base frequency"
    );
    // Each end of the bracket carries one flag's latency; over a run of
    // 140 million ticks both stay inside one part in ten thousand.
    let ratio = dec_ticks as f64 / tb_ticks as f64;
    assert!(
        (ratio - 1.0).abs() < 1e-4,
        "decrementer / time base = {ratio}"
    );

    // Phase 5 wrote 50000 and Tm fired as the count crossed zero.
    assert!(
        capture.record(5)[3] >= 0xFFFF_FFF0,
        "{:#x}",
        capture.record(5)[3]
    );

    // Phase 10: stopped across a spin after the masked acknowledgment,
    // running again after a write.
    let [stopped_a, stopped_b, written, after] = capture.record(10);
    assert_eq!(stopped_a, stopped_b, "the decrementer stopped");
    assert_eq!(written, 0x7FFF_FFFF, "a read right after the write");
    assert!(after < written, "and it counts down again");
}

fn wrch(channel: u8, value: u32, s: &mut SpuState) -> SpuStepOutcome {
    s.set_reg_word_splat(10, value);
    execute(&SpuInstruction::Wrch { channel, rt: 10 }, s, UnitId::new(0))
}

fn rdch(channel: u8, s: &mut SpuState) -> u32 {
    let out = execute(&SpuInstruction::Rdch { rt: 11, channel }, s, UnitId::new(0));
    assert_eq!(out, SpuStepOutcome::Continue, "rdch {channel}");
    s.reg_word(11)
}

fn count(channel: u8, s: &mut SpuState) -> u32 {
    let out = execute(
        &SpuInstruction::Rchcnt { rt: 12, channel },
        s,
        UnitId::new(0),
    );
    assert_eq!(out, SpuStepOutcome::Continue, "rchcnt {channel}");
    s.reg_word(12)
}

/// The probe's `quiet()`: the mask cleared, every event acknowledged.
fn quiet(s: &mut SpuState) {
    wrch(spu::SPU_WR_EVENT_MASK, 0, s);
    wrch(spu::SPU_WR_EVENT_ACK, u32::MAX, s);
}

#[test]
fn cellgov_raises_ms_as_the_console_does() {
    let console = Capture::load().record(1);
    let mut s = SpuState::new();
    quiet(&mut s);
    wrch(spu::SPU_WR_EVENT_MASK, spu::event::MS, &mut s);
    assert_eq!(
        wrch(spu::MFC_WR_MSSYNC_REQ, 0, &mut s),
        SpuStepOutcome::Continue
    );
    assert_eq!(count(spu::SPU_RD_EVENT_STAT, &mut s), 1, "pending at once");
    let status = rdch(spu::SPU_RD_EVENT_STAT, &mut s);
    wrch(spu::SPU_WR_EVENT_ACK, status, &mut s);
    let count_after = count(spu::SPU_RD_EVENT_STAT, &mut s);
    assert_eq!((status, count_after), (console[0], console[2]));
}

#[test]
fn cellgov_latches_a_masked_event_as_the_console_does() {
    let console = Capture::load().record(9);
    let mut s = SpuState::new();
    quiet(&mut s);
    wrch(spu::MFC_WR_MSSYNC_REQ, 0, &mut s);
    let masked = count(spu::SPU_RD_EVENT_STAT, &mut s);
    wrch(spu::SPU_WR_EVENT_MASK, spu::event::MS, &mut s);
    let enabled = count(spu::SPU_RD_EVENT_STAT, &mut s);
    let status = rdch(spu::SPU_RD_EVENT_STAT, &mut s);
    assert_eq!(
        (masked, enabled, status),
        (console[0], console[2], console[3])
    );
}

/// The guest tick at which the time base reads `tb`.
fn at_tb(tb: u64) -> GuestTicks {
    GuestTicks::new(
        u64::try_from((u128::from(tb) * 1_000_000_000).div_ceil(79_800_000)).expect("in range"),
    )
}

/// The probe's `wait_event` as the SPU compiler emits it at local-store
/// address [`WAIT_EVENT`] (`spu-objdump` of the built image). It holds
/// the poll loop of six instructions, then the stores of the record.
const WAIT_EVENT_WORDS: [u32; 56] = [
    0x01e0_0009,
    0x2100_1589,
    0x4100_0788,
    0x0020_0000,
    0x60a1_2008,
    0x3200_0180,
    0x7802_0496,
    0x2100_0f96,
    0x1c00_4489,
    0x01e0_000a,
    0x4020_007f,
    0x207f_fd8a,
    0x4080_020b,
    0x3ec1_018c,
    0x4100_078e,
    0x3882_c18d,
    0x60a1_200e,
    0x7803_848f,
    0xb203_448c,
    0x2882_c190,
    0x2100_040f,
    0x01a0_0011,
    0x0400_0892,
    0x3400_0193,
    0x3ec0_0194,
    0xb2a4_c894,
    0x2400_0195,
    0x21a0_0112,
    0x3580_0009,
    0x4020_007f,
    0x4020_007f,
    0x01e0_001b,
    0x4080_041c,
    0x3887_019d,
    0x3ec2_019e,
    0xb3e7_4d9e,
    0x2887_019f,
    0x3500_0000,
    0x4080_0217,
    0x3ec1_0198,
    0x3885_c199,
    0xb346_4498,
    0x2885_c19a,
    0x327f_f880,
    0x4020_007f,
    0x127f_f40a,
    0x4080_0204,
    0x3ec1_0186,
    0x4080_0005,
    0x0020_0000,
    0x4020_007f,
    0x3881_0182,
    0x4020_007f,
    0xb0e0_8286,
    0x2881_0187,
    0x327f_ef00,
];
const WAIT_EVENT: usize = 0x9d0;

/// Phase 5 around the call, in order:
///
/// 1. `quiet()`.
/// 2. Set the mask to Tm.
/// 3. Write 50000 to the decrementer.
/// 4. Call `wait_event` on the record at 0x2000.
/// 5. Read the decrementer into the word at 0x2010.
const PHASE_5: [u32; 13] = [
    0x4080_0004, // il $4,0
    0x21a0_0084, // wrch $ch1,$4
    0x40ff_ff85, // il $5,-1
    0x21a0_0105, // wrch $ch2,$5
    0x4080_1006, // il $6,0x20
    0x21a0_0086, // wrch $ch1,$6
    0x4261_a807, // ila $7,50000
    0x21a0_0387, // wrch $ch7,$7
    0x4210_0003, // ila $3,0x2000
    0x3301_3580, // brsl $0,wait_event
    0x01a0_0408, // rdch $8,$ch8
    0x2084_0208, // stqa $8,0x2010
    0x0000_2000, // stop 0x2000
];
const RECORD: usize = 0x2000;

/// Phase 5: the decrementer loaded with 50000 and Tm enabled, and the
/// compiled poll loop run through the execution unit. Tm fires as the
/// count passes zero, after the console's poll count to one part in a
/// thousand. Tm reads back as exactly its own bit and counts no more
/// once acknowledged. The decrementer then reads just below zero.
#[test]
fn cellgov_raises_tm_after_the_consoles_poll_count() {
    let console = Capture::load().record(5);
    let mut rt = cellgov_core::Runtime::new(
        cellgov_mem::GuestMemory::new(0x1000),
        cellgov_time::Budget::new(10_000),
        1_000,
    );
    let unit = rt.register_unit_with(|id| {
        let mut spu = cellgov_spu::SpuExecutionUnit::new(id);
        let ls = &mut spu.state_mut().ls;
        for (base, words) in [(0, &PHASE_5[..]), (WAIT_EVENT, &WAIT_EVENT_WORDS[..])] {
            for (i, word) in words.iter().enumerate() {
                ls[base + 4 * i..base + 4 * i + 4].copy_from_slice(&word.to_be_bytes());
            }
        }
        spu
    });
    while rt.registry().effective_status(unit) != Some(cellgov_exec::UnitStatus::Finished) {
        let step = rt.step().expect("the SPU runs");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    let ls = &rt
        .registry()
        .get(unit)
        .and_then(|unit| {
            unit.as_any()
                .downcast_ref::<cellgov_spu::SpuExecutionUnit>()
        })
        .expect("the SPU unit")
        .state()
        .ls;
    let word = |at: usize| u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes"));
    let [status, polls, count_after, after] =
        [RECORD, RECORD + 4, RECORD + 8, RECORD + 16].map(word);
    assert_eq!((status, count_after), (console[0], console[2]));
    let off = f64::from(polls) / f64::from(console[1]) - 1.0;
    assert!(
        off.abs() < 1e-3,
        "{polls} polls against the console's {}",
        console[1]
    );
    assert!(
        console[3] >= 0xFFFF_FFF0 && after >= 0xFFFF_FFF0,
        "{after:#x}"
    );
}

/// Phase 10: phase 9 acknowledged every event with the mask clear, so
/// the decrementer holds across a spin; a write of 0x7FFFFFFF reads
/// back at once and then counts down.
#[test]
fn cellgov_stops_the_decrementer_on_a_masked_tm_acknowledgment_as_the_console_does() {
    let console = Capture::load().record(10);
    let mut s = SpuState::new();
    s.set_clock(at_tb(1_000));
    wrch(spu::SPU_WR_DEC, 50_000, &mut s);
    quiet(&mut s);
    let stopped_a = rdch(spu::SPU_RD_DEC, &mut s);
    s.set_clock(at_tb(1_000_000));
    let stopped_b = rdch(spu::SPU_RD_DEC, &mut s);
    wrch(spu::SPU_WR_DEC, 0x7FFF_FFFF, &mut s);
    let written = rdch(spu::SPU_RD_DEC, &mut s);
    s.set_clock(at_tb(2_000_000));
    let after = rdch(spu::SPU_RD_DEC, &mut s);
    assert_eq!(stopped_a == stopped_b, console[0] == console[1], "stopped");
    assert_eq!(written, console[2], "a read right after the write");
    assert_eq!(after < written, console[3] < console[2], "counting again");
}
