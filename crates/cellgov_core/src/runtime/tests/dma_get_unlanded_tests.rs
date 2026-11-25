//! The runtime names a completed MFC get that finds no issuer to land in.

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::state::Runtime;

fn span(start: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), 4).expect("fits")
}

#[test]
fn a_get_with_no_issuer_logs_its_unlanded_break_as_one_line() {
    let mut rt = Runtime::new(GuestMemory::new(0x200), Budget::new(4), 100);
    let get = DmaRequest::new(DmaDirection::Get, span(0x100), span(0x40), UnitId::new(7))
        .expect("equal lengths");
    rt.dma_queue
        .enqueue(DmaCompletion::new(get, GuestTicks::new(50)), None);

    rt.drain_pending_dma();

    assert_eq!(
        rt.lv2_host()
            .observability()
            .first_invariant_break_line()
            .as_deref(),
        Some(
            "lv2 host invariant break at runtime.dma_get_unlanded: UnitId(7): a completed \
             MFC get of 4 bytes to local store 0x00000040 did not land (UnknownUnit); the \
             issuer checked the range at issue (the first of 1)"
        ),
    );
}
