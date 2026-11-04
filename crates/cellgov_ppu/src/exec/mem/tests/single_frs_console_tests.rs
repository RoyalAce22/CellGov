//! `single_frs` held to the `ppu_stfs_select` capture: the word the
//! reference console's stfs wrote for each probe operand.

use super::*;

/// The probe's operands, in payload order.
const OPERANDS: [u64; 12] = [
    0x7E37_E43C_8800_759C,
    0xFE37_E43C_8800_759C,
    0x7FEF_FFFF_FFFF_FFFF,
    0x3FF0_0000_3000_0000,
    0x3FF0_0000_3FFF_FFFF,
    0x380F_FFFF_FFFF_FFFF,
    0x375F_FFFF_FFFF_FFFF,
    0x36AF_FFFF_FFFF_FFFF,
    0xB70F_FFFF_FFFF_FFFF,
    0x369F_FFFF_FFFF_FFFF,
    0x0000_0000_0000_0001,
    0x8010_0000_0000_0000,
];

#[test]
fn every_operand_stores_the_word_the_console_stored() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_stfs_select/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words.len(), 2 + OPERANDS.len());
    assert_eq!(words[0], 0, "the probe ran to its end");
    for (c, &d) in OPERANDS.iter().enumerate() {
        assert_eq!(single_frs(d), words[2 + c], "case {c}: 0x{d:016x}");
    }
}
