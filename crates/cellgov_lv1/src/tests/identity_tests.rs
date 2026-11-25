use super::*;
use cellgov_ps3_abi::lv1::uart::game_os_dispatcher_reply;

#[test]
fn the_dispatcher_data_hold_both_codes_as_big_endian_halfwords_at_2_and_4() {
    let identity = ConsoleIdentity {
        product_code: 0x1234,
        product_sub_code: 0x5678,
        ..SYNTHETIC_CONSOLE_IDENTITY
    };
    assert_eq!(
        identity.dispatcher_data(),
        [0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0]
    );
}

/// The kernel compares the product code and data bytes 8 to 15 of the
/// device type with the same codes
/// [OFW-4.93 lv2_kernel a:0x800000000022a10c-0x800000000022a118]
/// [OFW-4.93 lv2_kernel a:0x800000000022a024-0x800000000022a07c].
#[test]
fn the_synthetic_product_code_is_the_device_type_that_cellgov_serves_for_request_0x19002() {
    let device_type = game_os_dispatcher_reply("4.93", 0x19000, 0x19002)
        .expect("a source gives the reply to request 0x19002")
        .data;
    let low: [u8; 8] = device_type[8..].try_into().unwrap();
    assert_eq!(
        u64::from_be_bytes(low),
        u64::from(SYNTHETIC_CONSOLE_IDENTITY.product_code)
    );
}

/// The kernel tests the range 1 to 12
/// [OFW-4.93 lv2_kernel a:0x8000000000058a7c-0x8000000000058a88].
#[test]
fn the_synthetic_product_sub_code_is_in_the_range_1_to_12() {
    assert!((1..=12).contains(&SYNTHETIC_CONSOLE_IDENTITY.product_sub_code));
}
