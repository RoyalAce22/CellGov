//! The measured model charges each measured size its table time, sizes
//! between linearly, an off-line transfer its own table, and a transfer
//! no MFC command issued the fixed latency.

use super::*;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

const FALLBACK: u64 = 10;
const NOW: GuestTicks = GuestTicks::new(1_000);

fn request(direction: DmaDirection, main: u64, ls: u64, size: u64) -> DmaRequest {
    let main = ByteRange::new(GuestAddr::new(main), size).expect("main range");
    let ls = ByteRange::new(GuestAddr::new(ls), size).expect("ls range");
    let (source, destination) = match direction {
        DmaDirection::Get => (main, ls),
        DmaDirection::Put => (ls, main),
    };
    DmaRequest::new(direction, source, destination, UnitId::new(1)).expect("a request")
}

fn charged(req: &DmaRequest) -> u64 {
    let model = MeasuredMfcLatency::new(FALLBACK);
    model.completion_time(req, NOW, &DmaQueue::new()).raw() - NOW.raw()
}

#[test]
fn a_measured_size_takes_its_table_time_less_the_tag_wait() {
    for (i, &size) in MFC_TIMING_SIZES.iter().enumerate() {
        let get =
            request(DmaDirection::Get, 0x10_0000, 0x1000, u64::from(size)).with_command_word(0x40);
        let want = tb_to_ticks(
            u64::from(MFC_GET_LINE_TB_SUM[i] - MFC_TAG_WAIT_TB_SUM),
            u64::from(MFC_TIMING_RUNS),
        );
        assert_eq!(charged(&get), want, "{size} bytes");
    }
}

#[test]
fn a_size_between_two_measured_ones_takes_a_time_between_theirs() {
    let at = |size| measured_transfer_ticks(DmaDirection::Put, false, size);
    let (lo, mid, hi) = (at(4096), at(6144), at(8192));
    assert!(lo < mid && mid < hi, "{lo} {mid} {hi}");
}

#[test]
fn a_get_outlasts_a_put_and_an_off_line_transfer_takes_its_own_table() {
    let size = 2048;
    let get = request(DmaDirection::Get, 0x10_0000, 0x1000, size).with_command_word(0x40);
    let put = request(DmaDirection::Put, 0x10_0000, 0x1000, size).with_command_word(0x20);
    assert!(charged(&get) > charged(&put));
    let off = request(DmaDirection::Get, 0x10_0010, 0x1010, size).with_command_word(0x40);
    let i = MFC_TIMING_SIZES
        .iter()
        .position(|&s| u64::from(s) == size)
        .unwrap();
    assert_eq!(
        charged(&off),
        tb_to_ticks(
            u64::from(MFC_GET_OFF_LINE_TB_SUM[i] - MFC_TAG_WAIT_TB_SUM),
            u64::from(MFC_TIMING_RUNS),
        )
    );
}

#[test]
fn a_transfer_no_mfc_command_issued_keeps_the_fixed_latency() {
    let copy = request(DmaDirection::Put, 0x10_0000, 0x1000, 16384);
    assert_eq!(charged(&copy), FALLBACK);
}
