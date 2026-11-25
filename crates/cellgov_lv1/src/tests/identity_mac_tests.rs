use super::*;
use cellgov_ps3_abi::lv1::network::NET_CONTROL_MAC_ADDRESS_BITS;

/// The first byte of the synthetic MAC address.
fn first_byte() -> u8 {
    SYNTHETIC_CONSOLE_IDENTITY.mac_address.to_be_bytes()[2]
}

#[test]
fn the_synthetic_mac_address_fits_the_low_48_bits_of_r4() {
    assert_eq!(NET_CONTROL_MAC_ADDRESS_BITS, 48);
    assert_eq!(SYNTHETIC_CONSOLE_IDENTITY.mac_address >> 48, 0);
}

/// The kernel's function fails for an r4 of 0
/// [OFW-4.93 lv2_kernel a:0x80000000000a36c4-0x80000000000a36c8].
#[test]
fn the_synthetic_mac_address_is_not_0() {
    assert_ne!(SYNTHETIC_CONSOLE_IDENTITY.mac_address, 0);
}

/// In the IEEE 802 address format, bit 1 of the first byte marks a
/// locally administered address and bit 0 marks a group address.
#[test]
fn the_synthetic_mac_address_is_a_locally_administered_unicast_address() {
    assert_eq!(first_byte() & 0b11, 0b10);
}
