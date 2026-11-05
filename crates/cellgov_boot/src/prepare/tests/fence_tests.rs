//! A fenced title memory refuses everything below the text and fetches
//! only its declared code.

use cellgov_mem::{ByteRange, FetchRefusal, GuestAddr, GuestMemory};

use super::fence_title_memory;

#[test]
fn a_fenced_title_memory_refuses_the_null_page_and_fetches_only_code() {
    let mut mem = GuestMemory::new(0x4_0000);
    mem.declare_code(0x1_0000, 0x100);
    fence_title_memory(&mut mem);
    for ea in [0x0, 0x8000] {
        let load = ByteRange::new(GuestAddr::new(ea), 4).unwrap();
        assert_eq!(mem.read(load), None, "a load at 0x{ea:x}");
        assert_eq!(mem.fetch_span(ea), Err(FetchRefusal::Unmapped), "0x{ea:x}");
    }
    assert_eq!(mem.fetch_span(0x1_0000), Ok((0x1_0000, 0x1_0100)));
    assert_eq!(mem.fetch_span(0x1_0100), Err(FetchRefusal::NoExecute));
}
