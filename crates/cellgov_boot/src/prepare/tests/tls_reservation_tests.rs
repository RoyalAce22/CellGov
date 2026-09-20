use super::check_tls_reservation;
use crate::prx::{FirmwareLoadError, PrxLoadInfo, TLS_BASE};

fn module_ending_at(data_end: u64) -> PrxLoadInfo {
    PrxLoadInfo {
        name: "test.sprx".to_string(),
        stem: "test".to_string(),
        base: 0,
        data_end,
        toc: 0,
        relocs_applied: 0,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn a_firmware_set_crossing_tls_is_refused_as_a_region_size() {
    let error = check_tls_reservation(0x20_0000, &[module_ending_at(TLS_BASE + 1)])
        .expect_err("the firmware set overlaps TLS");
    assert!(matches!(
        error,
        FirmwareLoadError::RegionSize {
            image_end,
            tls_base
        } if image_end == TLS_BASE + 1 && tls_base == TLS_BASE
    ));
}

#[test]
fn a_title_image_crossing_tls_is_refused_as_a_region_size() {
    let error = check_tls_reservation((TLS_BASE + 1) as usize, &[])
        .expect_err("the title image overlaps TLS");
    assert!(matches!(error, FirmwareLoadError::RegionSize { .. }));
}

#[test]
fn a_resident_set_ending_at_tls_keeps_the_reservation_clear() {
    check_tls_reservation(TLS_BASE as usize, &[module_ending_at(TLS_BASE)])
        .expect("an exclusive end at the TLS base does not overlap it");
}
