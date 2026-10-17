//! Each head's flip-flags word after context allocation, held to the
//! driver-info block the `lv2_rsx_answers` capture read on the
//! reference console.

use cellgov_ps3_abi::lv2::rsx::{driver_info, driver_info_init};

use super::write_rsx_driver_info_init;
use crate::host::test_support::console_payload;

/// Where the probe's payload holds the driver-info block it read.
const DRIVER: usize = 0x1100;

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

#[test]
fn both_heads_start_with_the_console_flip_flags() {
    let payload = console_payload("lv2_rsx_answers");
    assert_eq!(u32_at(&payload, 0), 0, "rsxInit succeeded on the console");
    let console = &payload[DRIVER..DRIVER + driver_info::SIZE];
    let mut cellgov = vec![0u8; driver_info::SIZE];
    write_rsx_driver_info_init(&mut cellgov, driver_info_init::MEMORY_SIZE, 0, 1);
    for head in 0..driver_info::HEAD_COUNT {
        let at = driver_info::HEAD_FLIP_FLAGS_OFFSET + head * driver_info::HEAD_STRIDE;
        assert_eq!(u32_at(&cellgov, at), u32_at(console, at), "head {head}");
    }
}
