//! A region view whose `base` differs from its `real` address maps each
//! byte, and logs each read, at the `real` address.

use super::*;

/// The first view of `mem`, moved to `base` with its `real` address kept.
fn shifted(mem: &GuestMemory, base: u64) -> RegionView<'_> {
    let view = mem.region_views().next().expect("one region");
    RegionView { base, ..view }
}

fn zero_readable() -> GuestMemory {
    GuestMemory::from_regions(vec![Region::with_access(
        0x1000,
        0x100,
        "zero",
        PageSize::Page4K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap()
}

#[test]
fn a_view_of_the_memory_maps_each_byte_to_itself() {
    let mem = zero_readable();
    let view = mem.region_views().next().unwrap();
    assert_eq!(view.real, view.base);
    assert_eq!(view.real_of(0x1010), 0x1010);
    assert_eq!(RegionView::plain(0x40, &[]).real_of(0x44), 0x44);
}

#[test]
fn a_moved_view_logs_a_provisional_read_at_the_real_address() {
    let mem = zero_readable();
    let view = shifted(&mem, 0x8000_0000_0000_1000);
    assert_eq!(view.real_of(0x8000_0000_0000_1010), 0x1010);
    view.note_read(0x8000_0000_0000_1010, 4);
    assert_eq!(
        mem.drain_provisional_reads(),
        [ProvisionalRead {
            addr: 0x1010,
            len: 4,
            hits: 1,
        }]
    );
}
