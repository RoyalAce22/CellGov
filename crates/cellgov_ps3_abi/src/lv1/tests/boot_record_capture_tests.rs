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

#[test]
fn a_frame_of_the_expected_whole_pairs_parses() {
    let len = u32::try_from(whole()).expect("small");
    assert_eq!(frame_pairs("f", &frame(len, whole())).len(), READS);
}

#[test]
#[should_panic(expected = "f: 27 whole (address, doubleword) pairs")]
fn a_frame_with_a_trailing_partial_pair_is_refused() {
    let len = u32::try_from(whole() + 5).expect("small");
    frame_pairs("f", &frame(len, whole() + 5));
}

#[test]
#[should_panic(expected = "f: the frame length is the rest of the file")]
fn a_frame_cut_short_of_its_length_is_refused() {
    let len = u32::try_from(whole()).expect("small");
    frame_pairs("f", &frame(len, whole() - 1));
}

#[test]
#[should_panic(expected = "f: 3 bytes, short of the 8-byte frame header")]
fn a_file_shorter_than_the_header_names_the_file() {
    frame_pairs("f", b"CGO");
}
