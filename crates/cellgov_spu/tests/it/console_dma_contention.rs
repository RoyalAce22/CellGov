//! CellGov's MFC latency model against the `spu_dma_contention` console
//! capture: transfers queued back to back, lists by element count and
//! size, and one SPU's transfers beside zero to five SPUs streaming
//! 16 KB gets.
//!
//! Each case enqueues the probe's transfers through
//! `MeasuredMfcLatency` into one `DmaQueue`, all issued at one time, and
//! reads when the last completes. The console's time also holds one tag
//! wait, which the model adds from the capture's own.

use cellgov_dma::{
    DmaCompletion, DmaDirection, DmaLatencyModel, DmaQueue, DmaRequest, MeasuredMfcLatency,
};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{tb_to_ticks, GuestTicks};

const CONSOLE: &str = "../../tests/micro/spu_dma_contention/ps3/cech20-cex-493/observation.json";

/// The transfer counts each queue and list row times.
const COUNTS: [u64; 5] = [1, 2, 4, 8, 16];

/// The probe's 384-byte payload as big-endian words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 384);
    let w: Vec<u32> = p
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!((w[0], w[1]), (0, 0), "no step failed");
    for k in 0..6 {
        assert_eq!(w[2 + 2 * k], 2, "group {k} exited through its threads");
        assert_eq!(w[3 + 2 * k], 0, "group {k}'s exit status");
        assert_ne!(w[75 + 4 * k], u32::MAX, "group {k}'s streamers started");
    }
    assert_eq!((w[16], w[17]), (32, 8), "the run counts");
    w
}

/// Word `at`, a sum over `runs` runs, as guest ticks a run.
fn ticks(w: &[u32], at: usize, runs: u32) -> u64 {
    tb_to_ticks(u64::from(w[at]), u64::from(runs))
}

/// The capture's bare tag wait, in guest ticks.
fn tag_wait(w: &[u32]) -> u64 {
    ticks(w, 18, w[16])
}

fn request(direction: DmaDirection, issuer: u64, main: u64, ls: u64, size: u64) -> DmaRequest {
    let main = ByteRange::new(GuestAddr::new(main), size).expect("main");
    let ls = ByteRange::new(GuestAddr::new(ls), size).expect("local store");
    let (source, destination) = match direction {
        DmaDirection::Get => (main, ls),
        DmaDirection::Put => (ls, main),
    };
    DmaRequest::new(direction, source, destination, UnitId::new(issuer))
        .expect("a request")
        .with_command_word(if direction == DmaDirection::Get {
            0x40
        } else {
            0x20
        })
}

/// The time issuer 1's `n` transfers of `size` bytes take, issued at
/// once beside `streamers` other SPUs each with a 16 KB get in flight.
fn model(direction: DmaDirection, n: u64, size: u64, streamers: u64) -> u64 {
    let latency = MeasuredMfcLatency::new(32);
    let now = GuestTicks::new(1_000);
    let mut queue = DmaQueue::new();
    for s in 0..streamers {
        let get = request(
            DmaDirection::Get,
            10 + s,
            0x100_0000 + s * 0x4000,
            0,
            0x4000,
        );
        let done = GuestTicks::new(1_000_000);
        queue.enqueue(DmaCompletion::new(get, done), None);
    }
    let mut last = now;
    for i in 0..n {
        let off = if size >= 0x4000 { 0 } else { i * size };
        let req = request(direction, 1, 0x10_0000 + off, 0x1_0000 + off, size);
        let done = latency.completion_time(&req, now, &queue);
        queue.enqueue(DmaCompletion::new(req, done), None);
        last = last.max(done);
    }
    last.raw() - now.raw()
}

/// Whether `cellgov` lands within `share` of the console's `console`.
fn near(cellgov: u64, console: u64, share: f64) -> bool {
    (cellgov as f64 - console as f64).abs() <= share * console as f64
}

#[test]
fn queued_transfers_take_one_latency_and_each_transfer_s_bytes_in_turn() {
    let w = console_words();
    let wait = tag_wait(&w);
    for (row, direction, size) in [
        (20, DmaDirection::Get, 1024),
        (25, DmaDirection::Get, 0x4000),
        (30, DmaDirection::Put, 1024),
        (35, DmaDirection::Put, 0x4000),
    ] {
        for (i, &n) in COUNTS.iter().enumerate().skip(2) {
            let console = ticks(&w, row + i, w[17]);
            let cellgov = model(direction, n, size, 0) + wait;
            assert!(
                near(cellgov, console, 0.12),
                "{n} x {size} B {direction:?}: CellGov {cellgov}, the console {console}",
            );
        }
        // Sixteen take near sixteen times one's bytes: the queue
        // serializes, on the console and in the model.
        let one = ticks(&w, row, w[17]);
        let sixteen = ticks(&w, row + 4, w[17]);
        assert!(
            sixteen > 4 * one,
            "the console, {size} B {direction:?}: {one} then {sixteen}"
        );
        let (one, sixteen) = (model(direction, 1, size, 0), model(direction, 16, size, 0));
        assert!(
            sixteen > 4 * one,
            "CellGov, {size} B {direction:?}: {one} then {sixteen}"
        );
    }
}

#[test]
fn a_list_takes_the_time_of_its_elements_queued_back_to_back() {
    let w = console_words();
    let wait = tag_wait(&w);
    for (row, direction, size) in [
        (40, DmaDirection::Get, 128),
        (45, DmaDirection::Get, 1024),
        (50, DmaDirection::Get, 4096),
        (55, DmaDirection::Put, 1024),
    ] {
        for (i, &n) in COUNTS.iter().enumerate().skip(2) {
            let console = ticks(&w, row + i, w[17]);
            let cellgov = model(direction, n, size, 0) + wait;
            assert!(
                near(cellgov, console, 0.15),
                "a list of {n} x {size} B {direction:?}: CellGov {cellgov}, the console {console}",
            );
        }
    }
}

#[test]
fn beside_streaming_spus_a_large_transfer_takes_its_share_of_the_bus() {
    let w = console_words();
    let wait = tag_wait(&w);
    for k in 0..6u64 {
        let block = 72 + 4 * k as usize;
        let single = ticks(&w, block, w[16]);
        let cellgov = model(DmaDirection::Get, 1, 0x4000, k) + wait;
        assert!(
            near(cellgov, single, 0.10),
            "a 16 KB get beside {k} streamers: CellGov {cellgov}, the console {single}",
        );
        let queue = ticks(&w, block + 1, w[17]);
        let cellgov = model(DmaDirection::Get, 16, 4096, k) + wait;
        assert!(
            near(cellgov, queue, 0.12),
            "sixteen 4 KB gets beside {k} streamers: CellGov {cellgov}, the console {queue}",
        );
    }
}

#[test]
fn beside_five_streaming_spus_a_small_transfer_waits_longer_than_its_share() {
    // An accepted gap: a lone 1 KB get beside five streamers takes near
    // three times its time alone on the console, behind the other SPUs'
    // 16 KB transfers. The model charges it a get's latency and its share
    // of the bus, about 30 ticks against the console's 52.
    let w = console_words();
    let wait = tag_wait(&w);
    let alone = ticks(&w, 72 + 2, w[16]);
    let console = ticks(&w, 72 + 4 * 5 + 2, w[16]);
    assert!(console > 2 * alone, "the console: {alone} then {console}");
    let cellgov = model(DmaDirection::Get, 1, 1024, 5) + wait;
    assert!(
        cellgov < console * 3 / 4,
        "CellGov {cellgov}, the console {console}"
    );
}

#[test]
fn on_the_console_loads_and_stores_beside_queued_transfers_lose_a_few_percent() {
    // An accepted gap, pinned as a console fact: the loads and stores
    // slow by under 5% while sixteen transfers move through the local
    // store. The issue model times them as alone, which
    // `console_issue_runout` holds.
    let w = console_words();
    let alone_loads = f64::from(w[60]);
    for (loop_at, wait_at, alone) in [
        (61, 62, alone_loads),
        (64, 65, f64::from(w[63])),
        (66, 67, alone_loads),
        (68, 69, alone_loads),
    ] {
        let beside = f64::from(w[loop_at]);
        assert!(
            beside > alone && beside < 1.05 * alone,
            "word {loop_at}: {beside} against {alone} alone",
        );
        assert!(
            w[wait_at] != 0xEEEE_EEEE && w[wait_at] >= w[loop_at],
            "word {wait_at} recorded the transfers' end after the loop's start"
        );
    }
}
