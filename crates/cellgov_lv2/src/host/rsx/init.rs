//! Bit-exact initial-state writers for the reports and driver-info
//! sub-regions of an RSX context.

use cellgov_ps3_abi::sys_rsx::{driver_info, driver_info_init, reports};

/// Semaphore init sentinel pattern, repeated across all 1024 slots.
/// CellGov-picked debug-friendly bytes -- the actual PS3 init pattern
/// is not specified.
pub const SEMAPHORE_INIT_PATTERN: [u32; 4] = [0x1337_C0D3, 0x1337_BABE, 0x1337_BEEF, 0x1337_F001];

/// Fill `buf` with the bytes `sys_rsx_context_allocate` writes into
/// the driver-info region.
///
/// # Panics
///
/// Panics if `buf.len() != driver_info::SIZE`.
pub fn write_rsx_driver_info_init(
    buf: &mut [u8],
    memory_size: u32,
    system_mode: u32,
    handler_queue: u32,
) {
    assert_eq!(
        buf.len(),
        driver_info::SIZE,
        "write_rsx_driver_info_init expects an driver_info::SIZE-byte buffer"
    );
    buf.fill(0);
    let mut put = |offset: usize, value: u32| {
        buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    };
    put(0x00, driver_info_init::VERSION_DRIVER);
    put(0x04, driver_info_init::VERSION_GPU);
    put(0x08, memory_size);
    put(0x0C, driver_info_init::HARDWARE_CHANNEL);
    put(0x10, driver_info_init::NVCORE_FREQUENCY);
    put(0x14, driver_info_init::MEMORY_FREQUENCY);
    put(0x2C, driver_info_init::REPORTS_NOTIFY_OFFSET);
    put(0x30, driver_info_init::REPORTS_OFFSET_FIELD);
    put(0x34, driver_info_init::REPORTS_REPORT_OFFSET);
    put(0x50, system_mode);
    put(driver_info::HANDLER_QUEUE_OFFSET, handler_queue);
}

/// Fill `buf` with the bytes `sys_rsx_context_allocate` writes into
/// the reports region.
///
/// # Panics
///
/// Panics if `buf.len() != reports::SIZE`.
pub fn write_rsx_reports_init(buf: &mut [u8]) {
    assert_eq!(
        buf.len(),
        reports::SIZE,
        "write_rsx_reports_init expects an reports::SIZE-byte buffer"
    );
    buf.fill(0);

    for i in 0..1024 {
        let offset = i * 4;
        buf[offset..offset + 4].copy_from_slice(&SEMAPHORE_INIT_PATTERN[i % 4].to_be_bytes());
    }

    let ts_be = u64::MAX.to_be_bytes();
    for i in 0..64 {
        let offset = 0x1000 + i * 16;
        buf[offset..offset + 8].copy_from_slice(&ts_be);
    }

    let pad_be = u32::MAX.to_be_bytes();
    for i in 0..2048 {
        let offset = 0x1400 + i * 16;
        buf[offset..offset + 8].copy_from_slice(&ts_be);
        buf[offset + 12..offset + 16].copy_from_slice(&pad_be);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_rsx_reports_init_matches_rpcs3_pattern() {
        let mut expected = vec![0u8; reports::SIZE];
        for i in 0..1024 {
            let offset = i * 4;
            expected[offset..offset + 4]
                .copy_from_slice(&SEMAPHORE_INIT_PATTERN[i % 4].to_be_bytes());
        }
        for i in 0..64 {
            let offset = 0x1000 + i * 16;
            expected[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        }
        for i in 0..2048 {
            let offset = 0x1400 + i * 16;
            expected[offset..offset + 8].copy_from_slice(&u64::MAX.to_be_bytes());
            expected[offset + 12..offset + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        let mut actual = vec![0u8; reports::SIZE];
        write_rsx_reports_init(&mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    #[should_panic(expected = "reports::SIZE-byte buffer")]
    fn write_rsx_reports_init_rejects_wrong_size() {
        let mut buf = vec![0u8; 128];
        write_rsx_reports_init(&mut buf);
    }

    #[test]
    fn write_rsx_driver_info_init_stamps_all_fields() {
        let mut buf = vec![0u8; driver_info::SIZE];
        write_rsx_driver_info_init(&mut buf, 0x0F90_0000, 0xABCD, 0xE001);
        let read = |o: usize| u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
        assert_eq!(read(0x00), driver_info_init::VERSION_DRIVER);
        assert_eq!(read(0x04), driver_info_init::VERSION_GPU);
        assert_eq!(read(0x08), 0x0F90_0000);
        assert_eq!(read(0x0C), driver_info_init::HARDWARE_CHANNEL);
        assert_eq!(read(0x10), driver_info_init::NVCORE_FREQUENCY);
        assert_eq!(read(0x14), driver_info_init::MEMORY_FREQUENCY);
        assert_eq!(read(0x2C), driver_info_init::REPORTS_NOTIFY_OFFSET);
        assert_eq!(read(0x30), driver_info_init::REPORTS_OFFSET_FIELD);
        assert_eq!(read(0x34), driver_info_init::REPORTS_REPORT_OFFSET);
        assert_eq!(read(0x50), 0xABCD);
        assert_eq!(read(driver_info::HANDLER_QUEUE_OFFSET), 0xE001);
    }

    #[test]
    #[should_panic(expected = "driver_info::SIZE-byte buffer")]
    fn write_rsx_driver_info_init_rejects_wrong_size() {
        let mut buf = vec![0u8; 128];
        write_rsx_driver_info_init(&mut buf, 0, 0, 0);
    }
}
