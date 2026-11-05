//! The cache tags keep each line apart from every other line.

use super::cache::{Caches, Level};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The lines between two effective addresses whose line numbers agree in
/// their low 32 bits.
const ALIAS_STRIDE: u64 = 1 << 32;

#[test]
fn two_lines_two_to_the_32_apart_do_not_alias() {
    let mut caches = Caches::new();
    let low = 0x1000 * k::PPU_CACHE_LINE_BYTES;
    let high = low + ALIAS_STRIDE * k::PPU_CACHE_LINE_BYTES;
    assert_eq!(caches.load(low, 8, false), Level::Memory);
    assert_eq!(caches.load(high, 8, false), Level::Memory);
    assert_eq!(caches.load(low, 8, false), Level::L1);
    assert_eq!(caches.load(high, 8, false), Level::L1);
}

#[test]
fn line_0xffff_ffff_misses_an_empty_cache() {
    let mut caches = Caches::new();
    let ea = 0xFFFF_FFFF * k::PPU_CACHE_LINE_BYTES;
    assert_eq!(caches.load(ea, 8, false), Level::Memory);
    assert_eq!(caches.load(ea, 8, false), Level::L1);
}
