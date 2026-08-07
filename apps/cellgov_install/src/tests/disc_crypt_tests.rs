//! PS3 disc decryption: a golden (data1 -> key) vector pinning the
//! published constants, the real on-disc region header parsed from
//! genuine bytes, encrypt/decrypt round-trips (including the data1
//! wrapper), and the structural-rejection guards. No real encrypted
//! disc is on hand, so the key/region values are pinned to externally-
//! captured golden bytes rather than re-derived from this crate.

use super::*;
use aes::cipher::{
    block_padding::NoPadding, generic_array::GenericArray, BlockEncryptMut, KeyIvInit,
};

/// A 16-byte test `data1` and the content key an independent AES
/// (openssl 3.x `enc -aes-128-cbc -nopad` under DISC_KEY_SECRET /
/// DISC_KEY_IV) derives from it. Frozen as bytes so a fat-fingered
/// secret/IV constant makes the golden test go red.
const GOLDEN_DATA1: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
];
const GOLDEN_KEY: [u8; 16] = [
    0xCB, 0x4A, 0xF6, 0xE8, 0x25, 0x7E, 0x3F, 0xF4, 0xB2, 0xAB, 0x93, 0xE6, 0xDF, 0xDC, 0x08, 0x8C,
];

/// Build a synthetic disc image: the region table at offset 0, then
/// every sector filled with a recognizable pattern (sector 0 keeps its
/// region-table head).
fn build_image(regions: &[(u32, u32)], total_sectors: usize) -> Vec<u8> {
    let mut img = vec![0u8; total_sectors * SECTOR];
    img[0..4].copy_from_slice(&(regions.len() as u32).to_be_bytes());
    let mut off = 8;
    for (s, e) in regions {
        img[off..off + 4].copy_from_slice(&s.to_be_bytes());
        img[off + 4..off + 8].copy_from_slice(&e.to_be_bytes());
        off += 8;
    }
    for sec in 0..total_sectors {
        let base = sec * SECTOR;
        let start = if sec == 0 { off } else { base };
        for (i, byte) in img[start..base + SECTOR].iter_mut().enumerate() {
            *byte = ((sec * 7 + start + i) % 251) as u8;
        }
    }
    img
}

/// CBC-encrypt one sector in place under `key` with the production
/// per-sector IV (the inverse of the decrypt path).
fn encrypt_sector(key: &[u8; 16], sector: u64, block: &mut [u8]) {
    let iv = sector_iv(sector);
    let len = block.len();
    let _ = Aes128CbcEnc::new(GenericArray::from_slice(key), GenericArray::from_slice(&iv))
        .encrypt_padded_mut::<NoPadding>(block, len)
        .expect("sector is a multiple of 16 bytes");
}

/// Encrypt every protected sector of `plain` under `key`, returning the
/// synthetic encrypted image.
fn encrypt_image(plain: &[u8], key: &[u8; 16], regions: &[(u32, u32)]) -> Vec<u8> {
    let mut enc = plain.to_vec();
    let total = plain.len() / SECTOR;
    for sec in 0..total {
        let s = sec as u32;
        if regions.iter().any(|&(a, b)| a <= s && s <= b) {
            continue;
        }
        let base = sec * SECTOR;
        encrypt_sector(key, sec as u64, &mut enc[base..base + SECTOR]);
    }
    enc
}

#[test]
fn disc_key_derivation_matches_golden_vector() {
    // Pins the code to the published constants: openssl computed
    // GOLDEN_KEY from GOLDEN_DATA1 independently of this crate's AES
    // path. Reverting DISC_KEY_SECRET or DISC_KEY_IV makes this red
    // (the prior encrypt-then-decrypt round-trip was the identity for
    // any constants and caught nothing).
    assert_eq!(decrypt_disc_key(&GOLDEN_DATA1), GOLDEN_KEY);
}

#[test]
fn sector_iv_holds_big_endian_sector() {
    assert_eq!(sector_iv(0), [0u8; 16]);
    let iv = sector_iv(0x0102_0304_0506_0708);
    assert_eq!(&iv[0..8], &[0u8; 8]);
    assert_eq!(&iv[8..16], &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn parses_real_wipeout_region_header_bytes() {
    // The literal first 32 bytes of sector 0 of a real WipEout HD Fury
    // (BCES00664) image. The region table lives in the unprotected
    // region (sector 0), so it is cleartext on the encrypted image --
    // these are genuine on-disc bytes, not a re-encoding of our own
    // parser's output. count=3 followed by SIX values (not three)
    // structurally confirms the explicit (start,end)-pair layout.
    let header: [u8; 32] = [
        0x00, 0x00, 0x00, 0x03, // region_count = 3
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x5F, // (0, 0x75F)
        0x00, 0x0E, 0xCA, 0xC0, 0x00, 0x0E, 0xD0, 0x3F, // (0xECAC0, 0xED03F)
        0x00, 0x0E, 0xD4, 0xE0, 0x00, 0x10, 0xD4, 0xFF, // (0xED4E0, 0x10D4FF)
    ];
    let regions = read_unprotected_regions(&header).expect("parse real header");
    assert_eq!(
        regions,
        vec![(0, 0x75F), (0xECAC0, 0xED03F), (0xED4E0, 0x10D4FF)]
    );
}

#[test]
fn rejects_image_too_small_for_header() {
    assert!(matches!(
        read_unprotected_regions(&[0u8; 7]).unwrap_err(),
        DiscCryptError::TooSmall { .. }
    ));
}

#[test]
fn rejects_truncated_region_table() {
    let mut img = vec![0u8; 8];
    img[0..4].copy_from_slice(&5u32.to_be_bytes());
    assert!(matches!(
        read_unprotected_regions(&img).unwrap_err(),
        DiscCryptError::RegionTableTruncated { .. }
    ));
}

#[test]
fn rejects_too_many_regions() {
    let mut img = vec![0u8; 8];
    img[0..4].copy_from_slice(&(MAX_REGIONS + 1).to_be_bytes());
    assert!(matches!(
        read_unprotected_regions(&img).unwrap_err(),
        DiscCryptError::TooManyRegions { .. }
    ));
}

#[test]
fn rejects_inverted_region_range() {
    let mut img = vec![0u8; 16];
    img[0..4].copy_from_slice(&1u32.to_be_bytes()); // count = 1
    img[8..12].copy_from_slice(&5u32.to_be_bytes()); // start = 5
    img[12..16].copy_from_slice(&2u32.to_be_bytes()); // end = 2 (< start)
    assert!(matches!(
        read_unprotected_regions(&img).unwrap_err(),
        DiscCryptError::BadRegionRange { start: 5, end: 2 }
    ));
}

#[test]
fn rejects_non_sector_aligned_image() {
    // 2049 bytes: one whole sector plus one stray byte.
    let img = vec![0u8; SECTOR + 1];
    assert!(matches!(
        decrypt_disc_image_with_key(&img, &[0u8; 16]).unwrap_err(),
        DiscCryptError::ImageNotSectorAligned { len } if len == SECTOR + 1
    ));
}

#[test]
fn decrypt_round_trips_over_protected_sectors() {
    // Sector 0 (region table) and sectors 4-5 are plaintext; 1-3 are
    // protected. Drives the lower-level _with_key form.
    let regions = [(0u32, 0u32), (4u32, 5u32)];
    let plain = build_image(&regions, 6);
    let key = [0x42u8; 16];
    let enc = encrypt_image(&plain, &key, &regions);
    assert_ne!(enc, plain, "protected sectors were transformed");

    let dec = decrypt_disc_image_with_key(&enc, &key).expect("decrypt");
    assert_eq!(dec, plain, "decrypt recovers the original plaintext");
}

#[test]
fn decrypt_disc_image_round_trips_via_data1() {
    // Drives the real data1 -> key wrapper end to end (the _with_key
    // form alone never exercised decrypt_disc_key).
    let regions = [(0u32, 0u32), (3u32, 3u32)];
    let plain = build_image(&regions, 4);
    let key = decrypt_disc_key(&GOLDEN_DATA1);
    let enc = encrypt_image(&plain, &key, &regions);
    assert_ne!(enc, plain);

    let dec = decrypt_disc_image(&enc, &GOLDEN_DATA1).expect("decrypt via data1");
    assert_eq!(dec, plain);
}
