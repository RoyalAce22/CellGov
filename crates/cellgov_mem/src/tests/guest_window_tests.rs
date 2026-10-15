use crate::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region, RegionAccess};

const RESERVED_BASE: u64 = 0x10_0000;
const RESERVED_SIZE: usize = 0x40_0000;
const WINDOW: u64 = RESERVED_BASE + 0x10_0000;
const WINDOW_SIZE: usize = 0x10_0000;

fn memory() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED_BASE,
            RESERVED_SIZE,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap()
}

fn layout(mem: &GuestMemory) -> Vec<(u64, u64, &'static str, RegionAccess)> {
    mem.regions()
        .map(|r| (r.base(), r.size(), r.label(), r.access()))
        .collect()
}

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).unwrap()
}

#[test]
fn a_window_splits_the_reserved_region_around_it() {
    let mut mem = memory();
    mem.map_window(WINDOW, WINDOW_SIZE, "window").unwrap();
    let reserved = RegionAccess::ReservedZeroReadable;
    assert_eq!(
        layout(&mem)[1..],
        [
            (RESERVED_BASE, 0x10_0000, "reserved", reserved),
            (
                WINDOW,
                WINDOW_SIZE as u64,
                "window",
                RegionAccess::ReadWrite
            ),
            (WINDOW + WINDOW_SIZE as u64, 0x20_0000, "reserved", reserved),
        ]
    );
    mem.apply_commit(range(WINDOW + 4, 4), &[1, 2, 3, 4])
        .unwrap();
    assert_eq!(mem.read(range(WINDOW + 4, 4)), Some(&[1, 2, 3, 4][..]));
    assert!(matches!(
        mem.apply_commit(range(WINDOW - 4, 4), &[0; 4]),
        Err(MemError::ReservedWrite { .. })
    ));
}

#[test]
fn a_window_at_either_end_leaves_one_piece() {
    let mut mem = memory();
    mem.map_window(RESERVED_BASE, WINDOW_SIZE, "window")
        .unwrap();
    mem.map_window(
        RESERVED_BASE + (RESERVED_SIZE - WINDOW_SIZE) as u64,
        WINDOW_SIZE,
        "window",
    )
    .unwrap();
    assert_eq!(layout(&mem).len(), 4);
}

#[test]
fn unmapping_a_window_restores_the_region_and_its_hash() {
    let mut mem = memory();
    let before = (layout(&mem), mem.content_hash());
    mem.map_window(WINDOW, WINDOW_SIZE, "window").unwrap();
    mem.apply_commit(range(WINDOW, 4), &[9; 4]).unwrap();
    assert_ne!(mem.content_hash(), before.1);
    mem.unmap_window(WINDOW).unwrap();
    assert_eq!((layout(&mem), mem.content_hash()), before);
    assert_eq!(mem.read(range(WINDOW, 4)), Some(&[0; 4][..]));
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}

#[test]
fn unmapping_one_of_two_windows_keeps_the_other() {
    let mut mem = memory();
    let second = WINDOW + WINDOW_SIZE as u64;
    mem.map_window(WINDOW, WINDOW_SIZE, "window").unwrap();
    mem.map_window(second, WINDOW_SIZE, "window").unwrap();
    mem.unmap_window(WINDOW).unwrap();
    let reserved = RegionAccess::ReservedZeroReadable;
    assert_eq!(
        layout(&mem)[1..],
        [
            (RESERVED_BASE, 0x20_0000, "reserved", reserved),
            (
                second,
                WINDOW_SIZE as u64,
                "window",
                RegionAccess::ReadWrite
            ),
            (second + WINDOW_SIZE as u64, 0x10_0000, "reserved", reserved),
        ]
    );
    mem.unmap_window(second).unwrap();
    assert_eq!(layout(&mem), layout(&memory()));
}

#[test]
fn a_window_needs_a_whole_unwritten_reserved_region() {
    let mut mem = memory();
    assert_eq!(
        mem.map_window(0, 0x100, "window"),
        Err(MemError::OverlappingRegions),
        "a ReadWrite host"
    );
    assert!(matches!(
        mem.map_window(RESERVED_BASE + RESERVED_SIZE as u64 - 4, 8, "window"),
        Err(MemError::Unmapped(_))
    ));
    mem.map_window(WINDOW, WINDOW_SIZE, "window").unwrap();
    assert_eq!(
        mem.map_window(WINDOW, 4, "window"),
        Err(MemError::OverlappingRegions),
        "the window itself"
    );
}

#[test]
fn the_window_span_covers_every_mapped_window() {
    let mut mem = memory();
    assert_eq!(mem.window_span(), None);
    let second = WINDOW + 2 * WINDOW_SIZE as u64;
    mem.map_window(second, WINDOW_SIZE, "window").unwrap();
    assert_eq!(
        mem.window_span(),
        Some((second, second + WINDOW_SIZE as u64))
    );
    mem.map_window(WINDOW, WINDOW_SIZE, "window").unwrap();
    assert_eq!(
        mem.window_span(),
        Some((WINDOW, second + WINDOW_SIZE as u64))
    );
    mem.unmap_window(WINDOW).unwrap();
    mem.unmap_window(second).unwrap();
    assert_eq!(mem.window_span(), None);
}

#[test]
fn a_read_write_region_beside_a_reserved_one_is_no_window() {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(RESERVED_BASE - 0x1000, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED_BASE,
            RESERVED_SIZE,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    assert!(matches!(
        mem.unmap_window(RESERVED_BASE - 0x1000),
        Err(MemError::Unmapped(_))
    ));
}

#[test]
fn a_window_between_two_windows_unmaps() {
    let mut mem = memory();
    let windows = [RESERVED_BASE, WINDOW, WINDOW + WINDOW_SIZE as u64];
    for base in windows {
        mem.map_window(base, WINDOW_SIZE, "window").unwrap();
    }
    mem.unmap_window(WINDOW).unwrap();
    assert_eq!(
        layout(&mem)[2],
        (
            WINDOW,
            WINDOW_SIZE as u64,
            "reserved",
            RegionAccess::ReservedZeroReadable
        )
    );
    mem.unmap_window(windows[0]).unwrap();
    mem.unmap_window(windows[2]).unwrap();
    assert_eq!(layout(&mem), layout(&memory()));
    assert_eq!(mem.content_hash(), memory().content_hash());
}

#[test]
fn a_window_returns_to_its_own_region_beside_another_reserved_one() {
    let regions = || {
        GuestMemory::from_regions(vec![
            Region::with_access(
                0,
                RESERVED_BASE as usize,
                "other",
                PageSize::Page64K,
                RegionAccess::ReservedZeroReadable,
            ),
            Region::with_access(
                RESERVED_BASE,
                RESERVED_SIZE,
                "reserved",
                PageSize::Page64K,
                RegionAccess::ReservedZeroReadable,
            ),
        ])
        .unwrap()
    };
    let mut mem = regions();
    mem.map_window(RESERVED_BASE, WINDOW_SIZE, "window")
        .unwrap();
    mem.unmap_window(RESERVED_BASE).unwrap();
    assert_eq!(layout(&mem), layout(&regions()));
}

#[test]
fn only_a_mapped_window_unmaps() {
    let mut mem = memory();
    assert!(matches!(
        mem.unmap_window(WINDOW),
        Err(MemError::Unmapped(_))
    ));
    assert!(
        matches!(mem.unmap_window(0), Err(MemError::Unmapped(_))),
        "a region that is no window"
    );
}
