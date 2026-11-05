//! Which region a fetch reads, and the refusal for each region a fetch
//! cannot read.

use super::*;
use crate::{PageSize, Region, RegionAccess};

/// A memory with these regions:
/// - read-write at 0 and at 0x8000;
/// - zero-readable at 0x1_0000;
/// - strict reserved at 0x2_0000.
fn memory() -> GuestMemory {
    let reserved =
        |base, access| Region::with_access(base, 0x1000, "rsx", PageSize::Page4K, access);
    GuestMemory::from_regions(vec![
        Region::new(0, 0x4000, "main", PageSize::Page4K),
        Region::new(0x8000, 0x1000, "stack", PageSize::Page4K),
        reserved(0x1_0000, RegionAccess::ReservedZeroReadable),
        reserved(0x2_0000, RegionAccess::ReservedStrict),
    ])
    .unwrap()
}

#[test]
fn an_unrestricted_memory_fetches_the_region_that_holds_the_pc() {
    let mem = memory();
    let code = mem.fetch_code(0x8004).unwrap();
    assert_eq!(code.span, (0x8000, 0x9000));
    assert_eq!(code.base, 0x8000);
    assert_eq!(code.bytes.len(), 0x1000);
    assert_eq!(mem.fetch_span(0x10), Ok((0, 0x4000)));
}

#[test]
fn a_restricted_code_span_in_another_region_is_clipped_to_that_region() {
    let mut mem = memory();
    mem.declare_code(0x8800, 0x1000);
    mem.restrict_fetch();
    assert_eq!(mem.fetch_span(0x8800), Ok((0x8800, 0x9000)));
    assert_eq!(mem.fetch_span(0x87FC), Err(FetchRefusal::NoExecute));
}

#[test]
fn a_reserved_region_refuses_a_fetch_as_its_reads_do() {
    let mem = memory();
    assert_eq!(mem.fetch_span(0x1_0000), Err(FetchRefusal::NoExecute));
    assert_eq!(mem.fetch_span(0x2_0000), Err(FetchRefusal::Unmapped));
}

#[test]
fn a_window_refuses_a_fetch_as_no_execute() {
    let mut mem = memory();
    mem.map_window(0x1_0100, 0x100, "window").unwrap();
    assert_eq!(mem.fetch_span(0x1_0100), Err(FetchRefusal::NoExecute));
}
