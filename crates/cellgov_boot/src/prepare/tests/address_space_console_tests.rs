//! The boot's address-space regions held to the `lv2_address_space`
//! capture from the reference console: the lowest page a game's image
//! takes, and where an RSX context's local memory maps and what a write
//! there does.

use super::rsx_access;
use cellgov_mem::RegionAccess;
use cellgov_ps3_abi::hw::address_space::{PS3_RSX_BASE, PS3_RSX_SIZE, PS3_USER_TEXT_FLOOR};

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_address_space/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The pages the probe asks about before any RSX call, in payload order.
const EARLY_PAGES: [u64; 5] = [0x0, 0x8000, 0xF000, 0x1_0000, 0x4000_0000];

#[test]
fn nothing_maps_below_the_text_floor_and_the_image_starts_at_it_as_on_the_console() {
    let w = words();
    assert_eq!(w[0], 0, "the probe ran to its end");
    let mut checked = 0;
    for (k, &page) in EARLY_PAGES.iter().enumerate() {
        let mapped = w[2 + 3 * k] == 0;
        if page < PS3_USER_TEXT_FLOOR {
            assert!(
                !mapped,
                "{page:#x} is below the floor and mapped on the console"
            );
            checked += 1;
        } else if page == PS3_USER_TEXT_FLOOR {
            assert!(mapped, "the floor {page:#x} is unmapped on the console");
            checked += 1;
        }
    }
    assert_eq!(
        checked, 4,
        "three pages below the floor and the floor itself"
    );
}

#[test]
fn rsx_local_memory_maps_inside_the_rsx_region_as_on_the_console() {
    let w = words();
    assert_eq!(w[41], 0, "rsxInit");
    assert_eq!(u64::from(w[42]), PS3_RSX_BASE, "local memory base");
    assert!(
        u64::from(w[43]) <= PS3_RSX_SIZE as u64,
        "local size {:#x} fits the region",
        w[43]
    );
    assert_eq!(w[45], 0, "the local base is mapped");
}

/// The console maps RSX local memory read-write once a context is
/// allocated: a word written there reads back. CellGov keeps the region
/// reserved unless the title's manifest opts into the RSX mirror.
#[test]
fn a_write_to_rsx_local_memory_stores_on_the_console_and_needs_the_mirror_here() {
    let w = words();
    assert_eq!(w[49], 0xC0DE_1569, "the console: the write stored");
    assert_ne!(
        rsx_access(false, false),
        RegionAccess::ReadWrite,
        "CellGov's default"
    );
    assert_eq!(
        rsx_access(false, true),
        RegionAccess::ReadWrite,
        "CellGov with the RSX mirror"
    );
}
