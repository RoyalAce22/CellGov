//! SPU instruction encoders checked against hand-assembled big-endian bytes.

use super::*;

#[test]
fn ilhu_encodes_correctly() {
    let inst = ilhu(3, 0x1337);
    assert_eq!(inst.to_be_bytes(), [0x41, 0x09, 0x9B, 0x83]);
}

#[test]
fn iohl_encodes_correctly() {
    let inst = iohl(3, 0xBAAD);
    assert_eq!(inst.to_be_bytes(), [0x60, 0xDD, 0x56, 0x83]);
}

#[test]
fn il_encodes_correctly() {
    let inst = il(3, 4);
    assert_eq!(inst.to_be_bytes(), [0x40, 0x80, 0x02, 0x03]);
}

#[test]
fn wrch_encodes_correctly() {
    let inst = wrch(16, 3);
    assert_eq!(inst.to_be_bytes(), [0x20, 0xA0, 0x08, 0x03]);
}

#[test]
fn rdch_encodes_correctly() {
    let inst = rdch(9, 24);
    assert_eq!(inst.to_be_bytes(), [0x01, 0xA0, 0x0C, 0x09]);
}
