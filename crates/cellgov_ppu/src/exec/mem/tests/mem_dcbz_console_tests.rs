//! `dcbz` held to the `ppu_granule_nan` capture from the reference
//! console: a 512-byte, 128-byte-aligned buffer filled with 0xEE and one
//! `dcbz` on buffer + 168.

use super::*;
use cellgov_ps3_abi::hw::ppu::DCBZ_BLOCK_BYTES;

const BASE: u64 = 0x1000;
const TARGET: u64 = 128 + 40;

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_granule_nan/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

#[test]
fn one_dcbz_zeroes_the_block_the_console_zeroes() {
    let w = words();
    let mut mem = vec![0xEEu8; 512];
    let mut s = PpuState::new();
    s.set_gpr(3, TARGET);
    s.set_gpr(4, BASE);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        BASE,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    for eff in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
            let start = (range.start().raw() - BASE) as usize;
            mem[start..start + range.length() as usize].copy_from_slice(bytes.bytes());
        }
    }
    let first = mem
        .iter()
        .position(|&b| b == 0)
        .map_or(u32::MAX, |i| i as u32);
    let zeroed = mem.iter().filter(|&&b| b == 0).count() as u32;
    assert_eq!(first, w[32], "first zeroed byte");
    assert_eq!(zeroed, w[33], "bytes zeroed");
    assert_eq!(w[34], 0, "the console zeroed nothing outside the block");
    assert_eq!(DCBZ_BLOCK_BYTES as u32, w[33], "the block size");
}
