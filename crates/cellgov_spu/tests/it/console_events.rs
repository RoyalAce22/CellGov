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

/// Phase 5: the decrementer loaded with 50000 and Tm enabled. Tm fires
/// as the count passes zero, reads back as exactly its own bit, counts
/// no more once acknowledged, and the decrementer then reads just below
/// zero.
///
/// The poll loop runs on CellGov's clock, one guest tick per instruction:
/// the poll count the console's records hold measures the console's
/// instruction timing, which that clock does not follow.
#[test]
fn cellgov_raises_tm_as_the_decrementer_passes_zero_as_the_console_does() {
    let console = Capture::load().record(5);
    let mut s = SpuState::new();
    let mut tick = at_tb(1_000);
    s.set_clock(tick);
    quiet(&mut s);
    wrch(spu::SPU_WR_EVENT_MASK, spu::event::TM, &mut s);
    wrch(spu::SPU_WR_DEC, 50_000, &mut s);
    // rchcnt, compare and branch: three ticks a poll.
    let mut polls = 0;
    loop {
        tick = GuestTicks::new(tick.raw() + 3);
        s.set_clock(tick);
        s.sample_decrementer();
        if count(spu::SPU_RD_EVENT_STAT, &mut s) != 0 {
            break;
        }
        polls += 1;
        assert!(polls < MAX_POLLS, "Tm never fired");
    }
    let status = rdch(spu::SPU_RD_EVENT_STAT, &mut s);
    wrch(spu::SPU_WR_EVENT_ACK, status, &mut s);
    let count_after = count(spu::SPU_RD_EVENT_STAT, &mut s);
    let after = rdch(spu::SPU_RD_DEC, &mut s);
    assert_eq!((status, count_after), (console[0], console[2]));
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
