//! What `Runtime::place_bytes` lands, which reservations it drops, and
//! the record it leaves behind.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::address_space::PS3_RSX_BASE;
use cellgov_sync::ReservedLine;
use cellgov_time::Budget;
use cellgov_trace::{HostWriter, TraceReader, TraceRecord};

use super::*;

const MAIN: u64 = 0;

fn build() -> Runtime {
    let memory = GuestMemory::from_regions(vec![
        Region::new(MAIN, 4096, "main", PageSize::Page64K),
        Region::with_access(
            PS3_RSX_BASE,
            256,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .expect("disjoint regions in ascending order");
    Runtime::new(memory, Budget::new(4), 100)
}

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("in-range test address")
}

/// The child space backs the same addresses as the boot space, so a
/// placement into the wrong space still lands somewhere.
fn build_with_child_space() -> (Runtime, AddressSpaceId) {
    let mut rt = build();
    let space = AddressSpaceId::new(1);
    rt.create_address_space(space)
        .expect("space 1 is free in a fresh runtime");
    rt.space_memory_mut(space)
        .expect("the space was just created")
        .install_region(MAIN, 4096, "child-main", PageSize::Page64K)
        .expect("an empty space has no region to overlap");
    (rt, space)
}

fn build_with_shared_view() -> (Runtime, AddressSpaceId) {
    let mut rt = build();
    let space = AddressSpaceId::new(1);
    rt.create_address_space(space)
        .expect("space 1 is free in a fresh runtime");
    rt.register_shared_mapping(
        0x8006_0100_0000_0020,
        0x40,
        &[(AddressSpaceId::BOOT, 0x2_0000), (space, 0x3_0000)],
    )
    .expect("both views sit clear of every installed region");
    (rt, space)
}

fn host_writes(rt: &Runtime) -> Vec<TraceRecord> {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("the runtime's own stream decodes"))
        .filter(|r| matches!(r, TraceRecord::HostWrite { .. }))
        .collect()
}

#[test]
fn a_placement_lands_its_bytes_in_the_boot_space() {
    let mut rt = build();

    rt.place_bytes(
        AddressSpaceId::BOOT,
        range(MAIN + 0x40, 4),
        &[0xDE, 0xAD, 0xBE, 0xEF],
    )
    .expect("a writable region accepts the placement");

    assert_eq!(
        &rt.memory().as_bytes()[0x40..0x44],
        &[0xDE, 0xAD, 0xBE, 0xEF]
    );
}

#[test]
fn a_placement_carries_no_unit_so_it_exempts_none() {
    let mut rt = build();
    let holder = UnitId::new(0);
    let other = UnitId::new(1);
    rt.reservations_mut()
        .insert_or_replace(holder, ReservedLine::containing(MAIN));
    rt.reservations_mut()
        .insert_or_replace(other, ReservedLine::containing(MAIN));

    let cleared = rt
        .place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    assert_eq!(cleared, 2);
    assert_eq!(rt.reservations().get(holder), None);
    assert_eq!(rt.reservations().get(other), None);
}

#[test]
fn a_placement_names_itself_in_the_trace() {
    let mut rt = build();

    rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 8), &[0xAB; 8])
        .expect("a writable region accepts the placement");

    assert_eq!(
        host_writes(&rt),
        vec![TraceRecord::HostWrite {
            writer: HostWriter::Placement,
            space: AddressSpaceId::BOOT.raw(),
            addr: MAIN,
            len: 8,
            reservations_cleared: 0,
        }]
    );
}

#[test]
fn a_refused_placement_lands_nothing_and_keeps_the_reservation() {
    let mut rt = build();
    let holder = UnitId::new(0);
    rt.reservations_mut()
        .insert_or_replace(holder, ReservedLine::containing(PS3_RSX_BASE));

    let err = rt.place_bytes(AddressSpaceId::BOOT, range(PS3_RSX_BASE, 4), &[0xAB; 4]);

    assert!(matches!(err, Err(MemError::ReservedWrite { .. })));
    assert_eq!(
        rt.reservations().get(holder),
        Some(ReservedLine::containing(PS3_RSX_BASE)),
    );
    assert!(host_writes(&rt).is_empty());
}

#[test]
fn a_placement_whose_bytes_do_not_fill_its_range_is_refused() {
    let mut rt = build();

    let err = rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 2]);

    assert!(matches!(err, Err(MemError::LengthMismatch)));
    assert_eq!(&rt.memory().as_bytes()[..4], &[0, 0, 0, 0]);
    assert!(host_writes(&rt).is_empty());
}

#[test]
fn a_placement_lands_in_the_space_it_names_and_nowhere_else() {
    let (mut rt, space) = build_with_child_space();

    rt.place_bytes(space, range(MAIN + 0x40, 4), &[0xDE, 0xAD, 0xBE, 0xEF])
        .expect("a writable region accepts the placement");

    assert_eq!(
        &rt.space_memory(space)
            .expect("the child space is live")
            .as_bytes()[0x40..0x44],
        &[0xDE, 0xAD, 0xBE, 0xEF]
    );
    assert_eq!(
        &rt.memory().as_bytes()[0x40..0x44],
        &[0, 0, 0, 0],
        "the boot space backs the same address and must stay untouched"
    );
}

#[test]
fn a_placement_sweeps_only_the_named_space_reservation_table() {
    let (mut rt, space) = build_with_child_space();
    let boot_holder = UnitId::new(0);
    let child_holder = UnitId::new(1);
    rt.reservations_mut()
        .insert_or_replace(boot_holder, ReservedLine::containing(MAIN));
    rt.space_reservations_mut(space)
        .expect("the child space is live")
        .insert_or_replace(child_holder, ReservedLine::containing(MAIN));

    let cleared = rt
        .place_bytes(space, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    assert_eq!(cleared, 1);
    assert_eq!(
        rt.space_reservations(space)
            .expect("the child space is live")
            .get(child_holder),
        None
    );
    assert_eq!(
        rt.reservations().get(boot_holder),
        Some(ReservedLine::containing(MAIN)),
        "the boot table holds a line at the same address and is not this space's"
    );
}

#[test]
fn a_placement_records_the_space_it_landed_in() {
    let (mut rt, space) = build_with_child_space();

    rt.place_bytes(space, range(MAIN, 8), &[0xAB; 8])
        .expect("a writable region accepts the placement");

    assert_eq!(
        host_writes(&rt),
        vec![TraceRecord::HostWrite {
            writer: HostWriter::Placement,
            space: space.raw(),
            addr: MAIN,
            len: 8,
            reservations_cleared: 0,
        }]
    );
}

#[test]
#[should_panic(expected = "commit targeted a space that was never created")]
fn a_placement_into_a_space_that_was_never_created_panics() {
    let mut rt = build();
    let _ = rt.place_bytes(AddressSpaceId::new(9), range(MAIN, 4), &[0xAB; 4]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "targets a shared view")]
fn a_placement_into_a_shared_view_is_trapped_in_debug() {
    let (mut rt, _space) = build_with_shared_view();
    let _ = rt.place_bytes(AddressSpaceId::BOOT, range(0x2_0000, 4), &[0xAB; 4]);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_placement_into_a_shared_view_is_a_named_invariant_break_in_release() {
    // Release builds have no debug_assert: the bytes land in the one
    // view, the sibling goes stale, and the only witness is the
    // runtime.place_bytes_targets_shared_view break.
    let (mut rt, space) = build_with_shared_view();
    let breaks_before = rt.lv2_host().observability().invariant_break_count;

    rt.place_bytes(AddressSpaceId::BOOT, range(0x2_0000, 4), &[0xAB; 4])
        .expect("the shared view's region accepts the placement");

    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks_before + 1
    );
    assert_eq!(
        rt.space_memory(space)
            .expect("the child space is live")
            .read(range(0x3_0000, 4))
            .expect("the sibling view is readable"),
        &[0u8; 4],
        "the sibling view stays stale; the break names the incoherence"
    );
}
