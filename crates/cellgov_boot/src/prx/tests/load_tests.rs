use std::path::Path;

use super::{
    default_prx_base, locate_and_parse_manifest, manifest_rel_path, page_align_up_u64,
    prx_base_from_value, FirmwareLoadError,
};

/// The refusal reason a rejected `CELLGOV_PRX_BASE` value carries.
fn refusal_reason(value: &str, code_floor: u32) -> String {
    match prx_base_from_value(value, code_floor) {
        Ok(base) => panic!("{value:?} was accepted as 0x{base:x}"),
        Err(FirmwareLoadError::PrxBase { reason, .. }) => reason,
        Err(other) => panic!("{value:?}: unexpected refusal {other}"),
    }
}

#[test]
fn page_align_up_rounds_to_the_next_4k_boundary() {
    for (addr, want) in [
        (0u64, 0u64),
        (1, 0x1000),
        (0xFFF, 0x1000),
        (0x1000, 0x1000),
        (0x1001, 0x2000),
        // The last page that still rounds up inside a u64.
        (0xFFFF_FFFF_FFFF_F000, 0xFFFF_FFFF_FFFF_F000),
    ] {
        assert_eq!(
            page_align_up_u64(addr).unwrap(),
            want,
            "0x{addr:016x} rounds to the wrong page"
        );
    }
}

#[test]
fn page_align_up_refuses_an_address_whose_round_up_overflows() {
    for addr in [0xFFFF_FFFF_FFFF_F001u64, 0xFFFF_FFFF_FFFF_FFFE, u64::MAX] {
        match page_align_up_u64(addr) {
            Err(FirmwareLoadError::PageAlignOverflow { addr: named }) => {
                assert_eq!(named, addr, "the refusal names the wrong address");
            }
            Ok(v) => panic!("0x{addr:016x} rounded to 0x{v:016x} instead of overflowing"),
            Err(other) => panic!("0x{addr:016x}: unexpected refusal {other}"),
        }
    }
}

#[test]
fn the_default_prx_base_is_the_first_64k_page_at_or_past_the_code_floor() {
    for (floor, want) in [
        (0u32, 0u64),
        (1, 0x1_0000),
        (0x1_0000, 0x1_0000),
        (0x1_0001, 0x2_0000),
        // The round-up cannot overflow a u64 from a u32 floor, so the
        // fallback has no refusal arm. It leaves the main-region bound
        // to the loader, so a floor this high still yields a base.
        (u32::MAX, 0x1_0000_0000),
    ] {
        assert_eq!(
            default_prx_base(floor),
            want,
            "code_floor 0x{floor:x} placed the set wrong"
        );
    }
}

#[test]
fn a_prx_base_override_accepts_either_hex_prefix_and_surrounding_space() {
    // Lower prefix with padding, upper prefix, and no prefix at all.
    for value in [" 0x30000000 ", "0X30000000", "30000000"] {
        assert_eq!(
            prx_base_from_value(value, 0x10_0000).unwrap(),
            0x3000_0000,
            "{value:?} did not parse"
        );
    }
}

#[test]
fn a_prx_base_override_that_is_not_hex_is_refused() {
    // Empty, prefix-only, non-hex, Rust's digit separator (which
    // from_str_radix does not accept), and past u64.
    for value in ["", "0x", "zzz", "0x1_0000", "FFFFFFFFFFFFFFFFF"] {
        assert!(
            refusal_reason(value, 0).contains("not a hex u64"),
            "{value:?} should be refused as non-hex"
        );
    }
}

#[test]
fn a_prx_base_override_must_be_64k_aligned() {
    for value in ["0x30001000", "0x30000001", "0x3000ffff"] {
        assert!(
            refusal_reason(value, 0).contains("64K-aligned"),
            "{value:?} should be refused as misaligned"
        );
    }
    assert_eq!(prx_base_from_value("0x30000000", 0).unwrap(), 0x3000_0000);
}

#[test]
fn a_prx_base_override_below_the_code_floor_is_refused() {
    assert!(refusal_reason("0x20000000", 0x3000_0000).contains("below code_floor"));
    // The floor itself is placeable: the base may equal it.
    assert_eq!(
        prx_base_from_value("0x30000000", 0x3000_0000).unwrap(),
        0x3000_0000
    );
}

#[test]
fn a_prx_base_override_outside_the_main_region_is_refused() {
    // The main region ends where the RSX iomap window begins, so the
    // first page at that boundary is already out of bounds.
    for value in ["0x40000000", "0xffff0000", "0xffffffffffff0000"] {
        assert!(
            refusal_reason(value, 0).contains("main region"),
            "{value:?} should be refused as outside the main region"
        );
    }
    assert_eq!(
        prx_base_from_value("0x3fff0000", 0).unwrap(),
        0x3FFF_0000,
        "the last 64K page of the main region is placeable"
    );
}

#[test]
fn a_manifest_relative_path_is_root_relative_with_forward_slashes() {
    let root = Path::new("store").join("firmware").join("3.55");
    let file = root.join("sys").join("external").join("libaudio.sprx");
    assert_eq!(
        manifest_rel_path(&root, &file).unwrap(),
        "sys/external/libaudio.sprx"
    );
}

#[test]
fn a_module_outside_the_manifest_root_is_refused() {
    let root = Path::new("store").join("firmware");
    let file = Path::new("elsewhere").join("libaudio.sprx");
    match manifest_rel_path(&root, &file) {
        Err(FirmwareLoadError::ModuleOutsideRoot { .. }) => {}
        Ok(rel) => panic!("a module outside the root resolved to {rel:?}"),
        Err(other) => panic!("unexpected refusal {other}"),
    }
}

#[test]
fn a_tree_with_no_firmware_manifest_is_refused() {
    // The walk covers the directory and two levels up, and never the
    // working directory, so no manifest anywhere in the repo can
    // satisfy it.
    let dir = Path::new("cellgov-absent-firmware-root")
        .join("sys")
        .join("external");
    match locate_and_parse_manifest(&dir) {
        Err(FirmwareLoadError::NoManifest { dir: named }) => {
            assert_eq!(named, dir, "the refusal names the wrong directory");
        }
        Ok((root, _)) => panic!("an absent tree resolved to root {}", root.display()),
        Err(other) => panic!("unexpected refusal {other}"),
    }
}
