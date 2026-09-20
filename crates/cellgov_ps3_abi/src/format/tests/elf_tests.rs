use super::function_descriptor::encode_ps3_packed_opd;

#[test]
fn encode_ps3_packed_opd_byte_pattern() {
    assert_eq!(
        encode_ps3_packed_opd(0xCAFE_BABE, 0xDEAD_BEEF),
        [0xCA, 0xFE, 0xBA, 0xBE, 0xDE, 0xAD, 0xBE, 0xEF],
    );
}

#[test]
fn encode_ps3_packed_opd_zero_toc_byte_pattern() {
    assert_eq!(
        encode_ps3_packed_opd(0x0000_FF00, 0),
        [0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00],
    );
}
