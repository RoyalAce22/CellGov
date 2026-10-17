//! SPU DMA timing as a retail console shows it: the `spu_dma_timing`
//! probe's committed capture, read from the decrementer at the
//! time-base frequency.
//!
//! The capture pins what the console does: a transfer's time grows with
//! its size, a get takes longer than a put of the same size, and the
//! atomic round trips take a few time-base ticks. CellGov charges every
//! transfer one fixed latency in guest ticks, and guest ticks are not the
//! time base, so the model does not take the table yet; the second test
//! names that difference.

use cellgov_core::DEFAULT_DMA_LATENCY_TICKS;
use cellgov_dma::{DmaDirection, DmaLatencyModel, DmaQueue, DmaRequest, FixedLatency};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

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

fn put_of(size: u64) -> DmaRequest {
    DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), size).expect("source"),
        ByteRange::new(GuestAddr::new(0x10_0000), size).expect("destination"),
        UnitId::new(1),
    )
    .expect("a request")
}

/// CellGov's model charges a 16 B and a 16 KB transfer alike, where the
/// console's time grows with size. The fixed latency is in guest ticks,
/// which do not yet run at the time base, so the measured table has no
/// unit to convert into; this test fails once the model charges by size,
/// and the model's test then takes the capture's table.
#[test]
fn cellgov_charges_every_size_one_fixed_latency_the_console_does_not() {
    let capture = Capture::load();
    let put = capture.table(120);
    assert_ne!(put[0], put[SIZES - 1]);

    let model = FixedLatency::new(DEFAULT_DMA_LATENCY_TICKS.raw());
    let queue = DmaQueue::new();
    let now = GuestTicks::new(100);
    let small = model.completion_time(&put_of(16), now, &queue);
    let large = model.completion_time(&put_of(16384), now, &queue);
    assert_eq!(small, large);
    assert_eq!(
        small,
        GuestTicks::new(100 + DEFAULT_DMA_LATENCY_TICKS.raw())
    );
}
