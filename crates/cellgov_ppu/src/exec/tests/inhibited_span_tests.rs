//! Caching-inhibited checks and loads at the two ends of the effective
//! address space.

use super::memory_helpers::{caching_inhibited, load_ze, LoadPort, Width};
use crate::store_buffer::StoreBuffer;
use cellgov_event::UnitId;
use cellgov_mem::RegionView;

const PAGE: usize = 0x1000;
const TOP_PAGE: u64 = u64::MAX - 0xFFF;

fn inhibited(base: u64, bytes: &[u8]) -> RegionView<'_> {
    RegionView {
        caching_inhibited: true,
        ..RegionView::plain(base, bytes)
    }
}

/// [PPC-Book1 p:49 s:3.3.6] A zero count stores no byte, so it has no operand to refuse.
#[test]
fn an_empty_range_holds_no_caching_inhibited_byte() {
    let low = [0u8; PAGE];
    let views = [inhibited(0, &low)];
    for ea in [0, 0x10, 0x8000, TOP_PAGE] {
        assert!(!caching_inhibited(&views, ea, 0), "{ea:#x}");
    }
    assert!(caching_inhibited(&views, 0x10, 1));
}

/// [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the effective address arithmetic wraps from 2^64 - 1 to 0.
#[test]
fn only_a_range_that_wraps_past_the_top_meets_an_inhibited_page_at_0() {
    let low = [0u8; PAGE];
    let views = [inhibited(0, &low)];
    assert!(!caching_inhibited(&views, TOP_PAGE + 0x101, 4));
    assert!(!caching_inhibited(&views, TOP_PAGE + 0xFFC, 4));
    assert!(caching_inhibited(&views, TOP_PAGE + 0xFFE, 4));
}

#[test]
fn a_load_from_a_view_at_real_address_0_reads_its_bytes() {
    let mem = [0x11, 0x22, 0x33, 0x44];
    let views = [RegionView::plain(0, &mem)];
    let store_buf = StoreBuffer::new();
    let mut effects = Vec::new();
    let mut port = LoadPort::new(&views, &store_buf, &mut effects, UnitId::new(0));
    assert_eq!(load_ze(&mut port, 0, Width::B4), Ok(0x1122_3344));
}
