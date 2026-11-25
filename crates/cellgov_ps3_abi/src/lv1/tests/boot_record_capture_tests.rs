use super::{frame_pairs, HEADER, PAIR, READS};

/// A frame whose header gives `len` and whose payload is `payload` zero
/// bytes.
fn frame(len: u32, payload: usize) -> Vec<u8> {
    let mut bytes = b"CGOV".to_vec();
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.resize(HEADER + payload, 0);
    bytes
}

fn whole() -> usize {
    READS * PAIR
}

/// The big-endian bytes of 0x0102030405060708, the first pair's address.
const ADDRESS_BYTES: &[u8] = b"\x01\x02\x03\x04\x05\x06\x07\x08";

/// The big-endian bytes of 0x1112131415161718, the first pair's
/// doubleword.
const VALUE_BYTES: &[u8] = b"\x11\x12\x13\x14\x15\x16\x17\x18";

#[test]
fn a_frame_of_the_expected_whole_pairs_parses_each_as_a_big_endian_address_then_doubleword() {
    let len = u32::try_from(whole()).expect("small");
    let mut bytes = frame(len, whole());
    bytes[HEADER..HEADER + 8].copy_from_slice(ADDRESS_BYTES);
    bytes[HEADER + 8..HEADER + PAIR].copy_from_slice(VALUE_BYTES);
    let pairs = frame_pairs("f", &bytes, READS);
    assert_eq!(pairs.len(), READS);
    assert_eq!(pairs[0], (0x0102_0304_0506_0708, 0x1112_1314_1516_1718));
    assert_eq!(pairs[1], (0, 0));
}

#[test]
#[should_panic(expected = "f: frame magic")]
fn a_frame_with_another_magic_is_refused() {
    let len = u32::try_from(whole()).expect("small");
    let mut bytes = frame(len, whole());
    bytes[..4].copy_from_slice(b"CGOW");
    frame_pairs("f", &bytes, READS);
}

#[test]
#[should_panic(expected = "f: 27 whole (address, doubleword) pairs")]
fn a_frame_with_a_trailing_partial_pair_is_refused() {
    let len = u32::try_from(whole() + 5).expect("small");
    frame_pairs("f", &frame(len, whole() + 5), READS);
}

#[test]
#[should_panic(expected = "f: the frame length is the rest of the file")]
fn a_frame_cut_short_of_its_length_is_refused() {
    let len = u32::try_from(whole()).expect("small");
    frame_pairs("f", &frame(len, whole() - 1), READS);
}

#[test]
#[should_panic(expected = "f: 3 bytes, short of the 8-byte frame header")]
fn a_file_shorter_than_the_header_names_the_file() {
    frame_pairs("f", b"CGO", READS);
}
