use super::{
    STORAGE_DEVICE_ID_MAX, STORAGE_FLASH_ALIGNMENT_MASK, STORAGE_FLASH_DEVICE_TYPE,
    STORAGE_NOTIFICATION_DEVICE_ID, STORAGE_WRITE_BAD_FLAGS_STATUS, STORAGE_WRITE_DENIED_STATUS,
    STORAGE_WRITE_FLASH_ALIGNMENT_STATUS, STORAGE_WRITE_NOT_READY_STATUS,
    STORAGE_WRITE_NO_DEVICE_STATUS, STORAGE_WRITE_NO_REGION_STATUS, STORAGE_WRITE_RESERVED_FLAGS,
    STORAGE_WRITE_SKIP_ACCESS_CHECK_FLAGS,
};

#[test]
fn each_refusal_status_keeps_its_linux_value() {
    assert_eq!(STORAGE_WRITE_NO_DEVICE_STATUS, -6);
    assert_eq!(STORAGE_WRITE_NOT_READY_STATUS, -11);
    assert_eq!(STORAGE_WRITE_BAD_FLAGS_STATUS, -17);
    assert_eq!(STORAGE_WRITE_FLASH_ALIGNMENT_STATUS, -17);
    assert_eq!(STORAGE_WRITE_NO_REGION_STATUS, -6);
    assert_eq!(STORAGE_WRITE_DENIED_STATUS, -4);
}

#[test]
fn the_write_flag_and_the_skip_bits_are_not_reserved() {
    // The 4.93 kernel's storage wrappers set request flag 0x8 for a write.
    assert_eq!(STORAGE_WRITE_RESERVED_FLAGS & 0x8, 0);
    assert_eq!(
        STORAGE_WRITE_RESERVED_FLAGS & STORAGE_WRITE_SKIP_ACCESS_CHECK_FLAGS,
        0
    );
}

#[test]
fn the_device_ids_run_from_0_to_6_and_the_notification_device_is_minus_1() {
    assert_eq!(STORAGE_DEVICE_ID_MAX, 6);
    assert_eq!(STORAGE_NOTIFICATION_DEVICE_ID as i64, -1);
}

#[test]
fn the_flash_device_is_type_14_and_aligns_to_512_sectors() {
    assert_eq!(STORAGE_FLASH_DEVICE_TYPE, 14);
    assert_eq!(STORAGE_FLASH_ALIGNMENT_MASK + 1, 512);
}
