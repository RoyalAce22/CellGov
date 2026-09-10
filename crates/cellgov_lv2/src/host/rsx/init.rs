//! Bit-exact initial-state writers for the reports and driver-info
//! sub-regions of an RSX context.

use cellgov_ps3_abi::lv2::rsx::{driver_info, driver_info_init, reports};

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
    put(
        driver_info::VERSION_DRIVER_OFFSET,
        driver_info_init::VERSION_DRIVER,
    );
    put(
        driver_info::VERSION_GPU_OFFSET,
        driver_info_init::VERSION_GPU,
    );
    put(driver_info::MEMORY_SIZE_OFFSET, memory_size);
    put(
        driver_info::HARDWARE_CHANNEL_OFFSET,
        driver_info_init::HARDWARE_CHANNEL,
    );
    put(
        driver_info::NVCORE_FREQUENCY_OFFSET,
        driver_info_init::NVCORE_FREQUENCY,
    );
    put(
        driver_info::MEMORY_FREQUENCY_OFFSET,
        driver_info_init::MEMORY_FREQUENCY,
    );
    put(
        driver_info::REPORTS_NOTIFY_OFFSET_FIELD,
        driver_info_init::REPORTS_NOTIFY_OFFSET,
    );
    put(
        driver_info::REPORTS_OFFSET_FIELD,
        driver_info_init::REPORTS_OFFSET_FIELD,
    );
    put(
        driver_info::REPORTS_REPORT_OFFSET_FIELD,
        driver_info_init::REPORTS_REPORT_OFFSET,
    );
    put(driver_info::SYSTEM_MODE_OFFSET, system_mode);
    put(driver_info::HANDLER_QUEUE_OFFSET, handler_queue);
}

/// Fill `buf` with the bytes `sys_rsx_context_allocate` writes into
/// the reports region.
///
/// The semaphore block keeps the zero fill. No public source states
/// what the kernel leaves there, so CellGov synthesises nothing for
/// it.
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

    let notify_base = driver_info_init::REPORTS_NOTIFY_OFFSET as usize;
    let report_base = driver_info_init::REPORTS_REPORT_OFFSET as usize;
    let ts_be = u64::MAX.to_be_bytes();
    for i in 0..reports::NOTIFY_COUNT {
        let at = notify_base + i * reports::ENTRY_SIZE;
        buf[at..at + reports::TIMESTAMP_SIZE].copy_from_slice(&ts_be);
    }

    let pad_be = u32::MAX.to_be_bytes();
    for i in 0..reports::REPORT_COUNT {
        let at = report_base + i * reports::ENTRY_SIZE;
        buf[at..at + reports::TIMESTAMP_SIZE].copy_from_slice(&ts_be);
        let pad = at + reports::REPORT_PAD_OFFSET;
        buf[pad..pad + reports::REPORT_PAD_SIZE].copy_from_slice(&pad_be);
    }
}

#[cfg(test)]
#[path = "tests/init_tests.rs"]
mod tests;
