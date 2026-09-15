use std::path::Path;

use super::{
    checked_prx_base, default_prx_base, load_firmware_set_bound, locate_and_parse_manifest,
    manifest_rel_path, page_align_up_u64, resolve_prx_base, FirmwareLoadError,
};

/// The refusal reason a rejected `prx_base` override carries.
fn refusal_reason(base: u64, code_floor: u32) -> String {
    match checked_prx_base(base, code_floor) {
        Ok(placed) => panic!("0x{base:x} was accepted as 0x{placed:x}"),
        Err(FirmwareLoadError::PrxBase {
            base: named,
            reason,
        }) => {
            assert_eq!(named, base, "the refusal names the wrong base");
            reason
        }
        Err(other) => panic!("0x{base:x}: unexpected refusal {other}"),
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
fn no_prx_base_override_places_the_set_at_the_default_base() {
    assert_eq!(resolve_prx_base(None, 0x1_0001).unwrap(), 0x2_0000);
}

#[test]
fn a_prx_base_override_replaces_the_default_base() {
    assert_eq!(
        resolve_prx_base(Some(0x3000_0000), 0x10_0000).unwrap(),
        0x3000_0000
    );
}

#[test]
fn a_prx_base_override_is_checked_before_it_replaces_the_default_base() {
    match resolve_prx_base(Some(0x3000_1000), 0x10_0000) {
        Err(FirmwareLoadError::PrxBase { base, reason }) => {
            assert_eq!(base, 0x3000_1000);
            assert!(reason.contains("64K-aligned"), "{reason}");
        }
        other => panic!("a misaligned override resolved: {other:?}"),
    }
    // A spawned child's floor sits past its own image, so the one
    // override is checked against each process's floor in turn.
    match resolve_prx_base(Some(0x3000_0000), 0x3001_0000) {
        Err(FirmwareLoadError::PrxBase { reason, .. }) => {
            assert!(reason.contains("below code_floor 0x30010000"), "{reason}");
        }
        other => panic!("an override below the floor it is given resolved: {other:?}"),
    }
}

#[test]
fn a_prx_base_override_must_be_64k_aligned() {
    for base in [0x3000_1000u64, 0x3000_0001, 0x3000_ffff] {
        assert!(
            refusal_reason(base, 0).contains("64K-aligned"),
            "0x{base:x} should be refused as misaligned"
        );
    }
    assert_eq!(checked_prx_base(0x3000_0000, 0).unwrap(), 0x3000_0000);
}

#[test]
fn a_prx_base_override_below_the_code_floor_is_refused() {
    assert!(refusal_reason(0x2000_0000, 0x3000_0000).contains("below code_floor"));
    // The floor itself is placeable: the base may equal it.
    assert_eq!(
        checked_prx_base(0x3000_0000, 0x3000_0000).unwrap(),
        0x3000_0000
    );
}

#[test]
fn a_prx_base_override_outside_the_main_region_is_refused() {
    // The main region ends where the RSX iomap window begins, so the
    // first page at that boundary is already out of bounds.
    for base in [0x4000_0000u64, 0xffff_0000, 0xffff_ffff_ffff_0000] {
        assert!(
            refusal_reason(base, 0).contains("main region"),
            "0x{base:x} should be refused as outside the main region"
        );
    }
    assert_eq!(
        checked_prx_base(0x3fff_0000, 0).unwrap(),
        0x3FFF_0000,
        "the last 64K page of the main region is placeable"
    );
}

#[test]
fn a_prx_base_refusal_names_the_flag_that_set_it() {
    let err = checked_prx_base(0x3000_1000, 0).unwrap_err().to_string();
    assert!(err.starts_with("--prx-base 0x0000000030001000: "), "{err}");
}

/// A sink that records each warn line and drops the other channels.
#[derive(Default)]
struct WarnLog(std::cell::RefCell<Vec<String>>);

impl crate::BootSink for WarnLog {
    fn note(&self, _line: &str) {}
    fn warn(&self, line: &str) {
        self.0.borrow_mut().push(line.to_string());
    }
    fn guest_text(&self, _text: &str) {}
}

/// A key source for a load that opens no module.
struct NoVault;

impl crate::KeyVaultSource for NoVault {
    fn vault_for(
        &self,
        _bytes: &[u8],
    ) -> Result<&cellgov_install::keys::KeyVault, &cellgov_install::keys::KeyVaultError> {
        unreachable!("a boot with no firmware directory decrypts no module")
    }
}

/// The warn lines a boot with no firmware directory reports.
fn warns_with_no_firmware_dir(prx_base: Option<u64>) -> Vec<String> {
    let mut mem = cellgov_mem::GuestMemory::new(0x1_0000);
    let log = WarnLog::default();
    let (loaded, identity, _) = load_firmware_set_bound(
        None,
        &[],
        &mut mem,
        0x1_0000,
        prx_base,
        false,
        &log,
        &NoVault,
    )
    .expect("no firmware directory loads an empty set");
    assert!(loaded.is_empty() && identity.is_none());
    log.0.into_inner()
}

#[test]
fn a_prx_base_override_with_no_firmware_to_place_is_reported() {
    let warns = warns_with_no_firmware_dir(Some(0x3000_0000));
    assert!(
        warns
            .iter()
            .any(|w| w.contains("prx_base=0x30000000") && w.contains("no effect")),
        "{warns:?}"
    );
}

#[test]
fn a_boot_with_no_firmware_and_no_prx_base_override_warns_nothing() {
    assert_eq!(warns_with_no_firmware_dir(None), Vec::<String>::new());
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
