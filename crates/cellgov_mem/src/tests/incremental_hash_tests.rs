//! The incremental content hash: its literal value, its agreement with a
//! full rehash through writes, resets, region installs and clones, and
//! the page-level structure the additive sum has to keep.

use super::*;
use crate::{ByteRange, GuestAddr};

fn write(mem: &mut GuestMemory, addr: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

fn assert_current(mem: &GuestMemory, what: &str) {
    mem.invalidate_content_hash();
    assert_eq!(
        mem.content_hash(),
        mem.content_hash_from_scratch(),
        "{what}"
    );
}

#[test]
fn content_hash_wire_format_golden() {
    let mut mem = GuestMemory::new(8192);
    write(&mut mem, 0x10, &[0xde, 0xad, 0xbe, 0xef]);
    write(&mut mem, 0x1ffc, &[1, 2, 3, 4]);
    assert_eq!(
        mem.content_hash(),
        0x5c0b_25a3_e305_3c9e,
        "written by hand in the same commit as the change that moves it"
    );
}

/// SplitMix64 over `state`.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[test]
fn the_incremental_hash_equals_a_full_rehash_through_random_writes() {
    let mut rng = 7;
    let mut mem = GuestMemory::new(64 * 1024);
    for i in 0..5_000 {
        let v = next(&mut rng);
        let len = 1 + (v % 64) as usize;
        let addr = (v >> 16) % (64 * 1024 - len as u64);
        let fill = if v & 0x100 != 0 { 0 } else { (v >> 8) as u8 };
        write(&mut mem, addr, &vec![fill; len]);
        if i % 3 == 0 {
            assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
        }
    }
    assert_current(&mem, "random writes");
}

#[test]
fn a_page_written_back_to_zeros_hashes_as_never_written() {
    let fresh = GuestMemory::new(16 * 1024);
    let mut mem = GuestMemory::new(16 * 1024);
    write(&mut mem, 0x2000, &[9; 16]);
    assert_ne!(mem.content_hash(), fresh.content_hash());
    write(&mut mem, 0x2000, &[0; 16]);
    assert_eq!(mem.content_hash(), fresh.content_hash());
}

#[test]
fn two_pages_that_exchange_contents_hash_differently() {
    let mut a = GuestMemory::new(16 * 1024);
    let mut b = GuestMemory::new(16 * 1024);
    write(&mut a, 0x0000, &[1; 8]);
    write(&mut a, 0x1000, &[2; 8]);
    write(&mut b, 0x0000, &[2; 8]);
    write(&mut b, 0x1000, &[1; 8]);
    assert_ne!(a.content_hash(), b.content_hash());
}

#[test]
fn every_one_bit_flip_in_a_page_hashes_distinctly() {
    let mut base = GuestMemory::new(8192);
    write(&mut base, 0x1000, &[0x5a; 4096]);
    let mut hashes = vec![base.content_hash()];
    for bit in 0..4096 * 8 {
        let mut m = base.clone();
        let byte = bit / 8;
        write(&mut m, 0x1000 + byte as u64, &[0x5a ^ (1 << (bit % 8))]);
        hashes.push(m.content_hash());
    }
    let n = hashes.len();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(hashes.len(), n, "{} collision(s)", n - hashes.len());
}

#[test]
fn a_clone_carries_the_page_terms() {
    let mut mem = GuestMemory::new(16 * 1024);
    write(&mut mem, 0x100, &[3; 32]);
    let before = mem.content_hash();
    let mut copy = mem.clone();
    write(&mut copy, 0x3000, &[4; 32]);
    assert_current(&copy, "clone after a write");
    assert_eq!(mem.content_hash(), before, "the original is untouched");
    assert_current(&mem, "original after the clone's write");
}

#[test]
fn a_reset_returns_the_hash_to_a_fresh_memory() {
    let fresh = GuestMemory::new(16 * 1024);
    let mut mem = GuestMemory::new(16 * 1024);
    write(&mut mem, 0x100, &[3; 32]);
    let _ = mem.content_hash();
    write(&mut mem, 0x2100, &[5; 32]);
    mem.reset_for_reuse();
    assert_eq!(mem.content_hash(), fresh.content_hash());
    assert_current(&mem, "after reset");
}

#[test]
fn an_installed_region_joins_the_hash() {
    let mut mem = GuestMemory::new(4096);
    write(&mut mem, 0x10, &[1; 4]);
    let before = mem.content_hash();
    mem.install_region(0x10_0000, 8192, "extra", PageSize::Page4K)
        .unwrap();
    assert_ne!(mem.content_hash(), before, "the region map moved");
    write(&mut mem, 0x10_1000, &[2; 4]);
    assert_current(&mem, "write into the installed region");
}

#[test]
fn a_region_installed_below_a_written_region_keeps_the_terms_aligned() {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(0x10_0000, 3 * 4096, "high", PageSize::Page4K)
        .unwrap();
    write(&mut mem, 0x10_2000, &[7; 8]);
    let _ = mem.content_hash();
    mem.install_region(0x8_0000, 4096, "mid", PageSize::Page4K)
        .unwrap();
    assert_current(&mem, "after an install below the written region");
    write(&mut mem, 0x8_0010, &[8; 8]);
    write(&mut mem, 0x10_2000, &[0; 8]);
    assert_current(&mem, "writes on both sides of the install");
}

#[test]
fn a_memory_rebuilt_from_written_regions_hashes_their_bytes() {
    let mut mem = GuestMemory::new(16 * 1024);
    write(&mut mem, 0x1100, &[6; 16]);
    let regions = mem.regions().cloned().collect();
    let rebuilt = GuestMemory::from_regions(regions).unwrap();
    assert_eq!(rebuilt.content_hash_from_scratch(), mem.content_hash());
    assert_eq!(rebuilt.content_hash(), mem.content_hash());
}

#[test]
fn a_short_last_page_and_word_hash_to_their_literal_value() {
    // 6007 bytes: page 1 is short, and its last word is 7 bytes.
    let mut mem = GuestMemory::new(6007);
    write(&mut mem, 0x1770, &[1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(
        mem.content_hash(),
        0x1b96_c824_4331_381b,
        "written by hand in the same commit as the change that moves it"
    );
}
