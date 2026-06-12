//! PPC64 instruction encoders checked against hand-assembled big-endian bytes.

use super::*;

#[test]
fn li_encodes_correctly() {
    let inst = li(3, 1);
    assert_eq!(inst.to_be_bytes(), [0x38, 0x60, 0x00, 0x01]);
}

#[test]
fn lis_encodes_correctly() {
    let inst = lis(3, 2);
    assert_eq!(inst.to_be_bytes(), [0x3C, 0x60, 0x00, 0x02]);
}

#[test]
fn ori_encodes_correctly() {
    let inst = ori(6, 6, 8);
    assert_eq!(inst.to_be_bytes(), [0x60, 0xC6, 0x00, 0x08]);
}

#[test]
fn sc_encodes_correctly() {
    let inst = sc();
    assert_eq!(inst.to_be_bytes(), [0x44, 0x00, 0x00, 0x02]);
}

#[test]
fn lwz_encodes_correctly() {
    let inst = lwz(7, 6, 0);
    assert_eq!(inst.to_be_bytes(), [0x80, 0xE6, 0x00, 0x00]);
}

#[test]
fn stw_encodes_correctly() {
    let inst = stw(7, 6, 4);
    assert_eq!(inst.to_be_bytes(), [0x90, 0xE6, 0x00, 0x04]);
}

#[test]
fn beq_negative_offset() {
    let inst = beq(-8);
    assert_eq!(inst.to_be_bytes()[0], 0x41);
}

#[test]
fn encode_produces_big_endian_bytes() {
    let code = encode(&[li(3, 1), sc()]);
    assert_eq!(code.len(), 8);
    assert_eq!(&code[0..4], &[0x38, 0x60, 0x00, 0x01]);
    assert_eq!(&code[4..8], &[0x44, 0x00, 0x00, 0x02]);
}
