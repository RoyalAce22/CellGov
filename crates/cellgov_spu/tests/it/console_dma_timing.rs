//! SPU DMA timing as a retail console shows it: the `spu_dma_timing`
//! probe's committed capture, read from the decrementer at the
//! time-base frequency.
//!
//! The capture pins what the console does: a transfer's time grows with
//! its size, a get takes longer than a put of the same size, and the
//! atomic round trips take a few time-base ticks. A boot's latency model
//! charges each MFC transfer those times, converted from the time base
//! to guest ticks, and a getllar its measured round trip.

use cellgov_core::DEFAULT_DMA_LATENCY_TICKS;
use cellgov_dma::{DmaDirection, DmaLatencyModel, DmaQueue, DmaRequest, MeasuredMfcLatency};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::mfc_timing;
use cellgov_time::{tb_to_ticks, GuestTicks};

const CONSOLE: &str = "../../tests/micro/spu_dma_timing/ps3/cech20-cex-493/observation.json";

/// The probe's sizes: 16 << i bytes for i in 0..SIZES.
const SIZES: usize = 11;
/// The first size from which the probe's times rise with every doubling;
/// below it, a transfer's own time is under the run-to-run spread.
const RISING_FROM: usize = 5;

/// The probe's 240-byte payload.
struct Capture(Vec<u8>);

impl Capture {
    fn load() -> Self {
        let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
            .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
        let capture = Self(observation.memory_regions[0].data.clone());
        assert_eq!(capture.0.len(), 240);
        assert_eq!((capture.word(0), capture.word(4)), (0, 0), "no step failed");
        assert_eq!(capture.word(24), 0, "every putllc succeeded");
        capture
    }

    fn word(&self, at: usize) -> u32 {
        u32::from_be_bytes(self.0[at..at + 4].try_into().expect("4 bytes"))
    }

    /// The runs each timing word sums.
    fn reps(&self) -> u32 {
        self.word(16)
    }

    /// One size table: get aligned at 32, get unaligned at 76, put aligned
    /// at 120, put unaligned at 164.
    fn table(&self, at: usize) -> Vec<u32> {
        (0..SIZES).map(|i| self.word(at + i * 4)).collect()
    }
}

#[test]
fn dma_time_on_the_console_grows_with_size_and_a_get_outlasts_a_put() {
    let capture = Capture::load();
    assert_eq!(capture.reps(), 32);
    let [get, get_unaligned, put, put_unaligned] = [32, 76, 120, 164].map(|at| capture.table(at));
    for table in [&get, &get_unaligned, &put, &put_unaligned] {
        assert!(
            table[RISING_FROM..].windows(2).all(|w| w[0] < w[1]),
            "time rises with every doubling from {} bytes: {table:?}",
            16 << RISING_FROM
        );
        assert!(
            table[SIZES - 1] > 4 * table[0],
            "16 KB takes several times 16 B: {table:?}"
        );
    }
    for i in 0..SIZES {
        assert!(get[i] > put[i], "a get outlasts a put at {} bytes", 16 << i);
    }
    // The lists and the atomic round trips: a 16 x 1 KB list is close to
    // one 16 KB transfer, and an atomic round trip is far below one.
    let [getl, putl, getl_4x4k, getllar, putllc, putlluc] =
        [208, 212, 216, 220, 224, 228].map(|at| capture.word(at));
    assert!(getl > get[SIZES - 1] && getl_4x4k > get[SIZES - 1]);
    assert!(putl > put[SIZES - 1]);
    for atomic in [getllar, putllc, putlluc] {
        assert!(atomic < put[0], "an atomic round trip is under a 16 B put");
    }
}

/// The tables CellGov's model reads are the capture's words.
#[test]
fn the_measured_tables_are_the_captures() {
    let capture = Capture::load();
    assert_eq!(capture.reps(), mfc_timing::MFC_TIMING_RUNS);
    assert_eq!(capture.word(20), mfc_timing::MFC_TAG_WAIT_TB_SUM);
    for (at, table) in [
        (32, &mfc_timing::MFC_GET_LINE_TB_SUM),
        (76, &mfc_timing::MFC_GET_OFF_LINE_TB_SUM),
        (120, &mfc_timing::MFC_PUT_LINE_TB_SUM),
        (164, &mfc_timing::MFC_PUT_OFF_LINE_TB_SUM),
    ] {
        assert_eq!(capture.table(at), table.to_vec(), "payload offset {at}");
    }
    assert_eq!(capture.word(220), mfc_timing::MFC_GETLLAR_TB_SUM);
    for (i, &size) in mfc_timing::MFC_TIMING_SIZES.iter().enumerate() {
        assert_eq!(size, 16 << i);
    }
}

fn request(direction: DmaDirection, main: u64, ls: u64, size: u64) -> DmaRequest {
    let main = ByteRange::new(GuestAddr::new(main), size).expect("main");
    let ls = ByteRange::new(GuestAddr::new(ls), size).expect("local store");
    let (source, destination) = match direction {
        DmaDirection::Get => (main, ls),
        DmaDirection::Put => (ls, main),
    };
    DmaRequest::new(direction, source, destination, UnitId::new(1))
        .expect("a request")
        .with_command_word(0x20)
}

/// A boot's latency model charges each transfer the capture measured:
/// its run mean less the bare tag wait, in guest ticks at one
/// nanosecond each.
#[test]
fn cellgov_charges_each_transfer_the_time_the_console_measures() {
    let capture = Capture::load();
    let wait = capture.word(20);
    let runs = u64::from(capture.reps());
    let model = MeasuredMfcLatency::new(DEFAULT_DMA_LATENCY_TICKS.raw());
    let now = GuestTicks::new(100);
    for (at, direction, off) in [
        (32, DmaDirection::Get, 0),
        (76, DmaDirection::Get, 16),
        (120, DmaDirection::Put, 0),
        (164, DmaDirection::Put, 16),
    ] {
        for (i, sum) in capture.table(at).into_iter().enumerate() {
            let size = 16u64 << i;
            let req = request(direction, 0x10_0000 + off, 0x1000 + off, size);
            let charged = model.completion_time(&req, now, &DmaQueue::new()).raw() - now.raw();
            assert_eq!(
                charged,
                tb_to_ticks(u64::from(sum - wait), runs),
                "offset {at}, {size} bytes"
            );
        }
    }
}

/// A getllar charges its unit the round trip the console measures, in
/// SPU cycles, whatever budget its step has left.
#[test]
fn a_getllar_takes_the_round_trip_the_console_measures() {
    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    use cellgov_mem::GuestMemory;
    use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GETLLAR};
    use cellgov_ps3_abi::hw::spu_issue::{SPU_CYCLES_PER_TB_TICK, SPU_GETLLAR_CYCLES};
    use cellgov_spu::SpuExecutionUnit;
    use cellgov_time::{spu_cycles_to_ticks, Budget};

    let capture = Capture::load();
    assert_eq!(
        SPU_GETLLAR_CYCLES * u64::from(capture.reps()),
        u64::from(capture.word(220)) * SPU_CYCLES_PER_TB_TICK,
        "the mean round trip, in cycles"
    );
    // `wrch MFC_Cmd, r4`, then `stop`.
    let program = [(0x10Du32 << 21) | (u32::from(MFC_CMD) << 7) | 4, 0];
    // The wrch's cycle, the trip, and the stop's cycle when the budget
    // reaches it.
    for (budget, cycles) in [
        (1_000, 1 + SPU_GETLLAR_CYCLES + 1),
        (2, 1 + SPU_GETLLAR_CYCLES + 1),
        (1, 1 + SPU_GETLLAR_CYCLES),
    ] {
        let mut unit = SpuExecutionUnit::new(UnitId::new(1));
        for (i, word) in program.iter().enumerate() {
            unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        unit.state_mut().channels.mfc_lsa = 0x100;
        unit.state_mut().channels.mfc_eal = 0x80;
        unit.state_mut().set_reg_word_splat(4, MFC_GETLLAR);
        let mem = GuestMemory::new(0x1000);
        let result = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        assert!(
            unit.state().reservation().is_some(),
            "budget {budget}: the getllar ran"
        );
        assert_eq!(
            result.consumed_cost.raw(),
            spu_cycles_to_ticks(cycles),
            "budget {budget}"
        );
    }
}
