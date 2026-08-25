//! PS3 encrypted-disc (3k3y/redump) decryption: turn an encrypted
//! BD-ROM image plus the user's disc key into a plaintext image the
//! ISO9660 reader ([`crate::iso`]) can walk.
//!
//! A PS3 disc splits its sectors into unprotected (plaintext) and
//! protected (AES-128-CBC encrypted) regions. The region table sits in
//! cleartext at the head of the image. The content key is derived from
//! the 16-byte `data1` -- the redump `.dkey` -- by a single-block
//! AES-128-CBC encryption under a fixed secret/IV; each protected
//! sector is then CBC-decrypted under that key with an IV carrying the
//! big-endian sector number.
//!
//! RPCS3 has no disc-decryption path, reference here is the open-source
//! PS3 Disc Dumper (`13xforever/ps3-disc-dumper`). Two files of that
//! repo are the sources: `Decrypter.cs` for the key derivation,
//! per-sector IV, and CBC decrypt (the fixed secret/IV are its
//! published constants), and `IrdLibraryClient/IrdFormat/
//! IsoHeaderParser.cs` `GetUnprotectedRegions` for the region-table
//! layout at the image head (big-endian count, a reserved word, then
//! `count` `(start, end)` sector pairs).

use aes::cipher::{
    block_padding::NoPadding, generic_array::GenericArray, BlockDecryptMut, BlockEncryptMut,
    KeyIvInit,
};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Logical sector size for PS3 BD-ROM data.
const SECTOR: usize = 2048;

/// Fixed AES-128 key that turns a disc's `data1` into its content key.
const DISC_KEY_SECRET: [u8; 16] = [
    CELLGOV-REDACTED-KEY,
];
/// Fixed IV paired with [`DISC_KEY_SECRET`].
const DISC_KEY_IV: [u8; 16] = [
    CELLGOV-REDACTED-KEY,
];

/// Hard ceiling on the region count read from the image head.
///
/// Sony publishes no maximum (the disc format spec is under NDA), but
/// the format itself imposes one: the region table lives entirely in
/// sector 0, which is [`SECTOR`] (2048) bytes. With the 8-byte header
/// (count + reserved word) and one 8-byte `(start, end)` pair per
/// region, at most `(2048 - 8) / 8 = 255` regions fit. A count above
/// 255 is therefore structurally malformed, not merely suspicious.
/// (Real discs carry 2-3 unprotected regions.)
const MAX_REGIONS: u32 = (SECTOR as u32 - 8) / 8; // = 255

/// Why decrypting an encrypted disc image failed.
#[derive(Debug, thiserror::Error)]
pub enum DiscCryptError {
    /// Image is too small to hold even the region-count header.
    #[error("disc image too small for region table (got {len} bytes)")]
    TooSmall {
        /// Observed image length.
        len: usize,
    },
    /// Image length is not a whole number of sectors. A real
    /// whole-image dump is always a sector multiple; anything else is
    /// rejected rather than silently leaving the trailing partial
    /// sector undecrypted.
    #[error("disc image length 0x{len:x} is not a multiple of the {sz}-byte sector", sz = SECTOR)]
    ImageNotSectorAligned {
        /// Observed image length.
        len: usize,
    },
    /// The region table declares more entries than the image holds.
    #[error("disc region table needs 0x{needed:x} bytes, image head is 0x{len:x}")]
    RegionTableTruncated {
        /// Bytes the region table would occupy.
        needed: usize,
        /// Actual image length.
        len: usize,
    },
    /// The region count cannot fit in sector 0 (not a valid PS3 disc).
    #[error("disc region count {count} exceeds the {max} cap", max = MAX_REGIONS)]
    TooManyRegions {
        /// The declared region count.
        count: u32,
    },
    /// A region's start sector is past its end sector.
    #[error("disc region [{start}, {end}] is inverted")]
    BadRegionRange {
        /// Region start sector.
        start: u32,
        /// Region end sector.
        end: u32,
    },
}

fn read_be_u32(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(
        data[off..off + 4]
            .try_into()
            .expect("invariant: caller bounds-checked this 4-byte read"),
    )
}

/// Derive the per-disc content key from the 16-byte `data1` (the
/// redump `.dkey`): a single-block AES-128-CBC encryption under the
/// fixed secret/IV.
#[must_use]
pub fn decrypt_disc_key(data1: &[u8; 16]) -> [u8; 16] {
    let mut block = *data1;
    let _ = Aes128CbcEnc::new(
        GenericArray::from_slice(&DISC_KEY_SECRET),
        GenericArray::from_slice(&DISC_KEY_IV),
    )
    .encrypt_padded_mut::<NoPadding>(&mut block, 16)
    .expect("invariant: a single 16-byte block needs no padding");
    block
}

/// The 16-byte CBC IV for a protected sector: zeros, then the
/// big-endian sector number in the low 8 bytes.
fn sector_iv(sector: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&sector.to_be_bytes());
    iv
}

/// Parse the unprotected (plaintext) sector ranges from the image
/// head. Layout (mirrors PS3 Disc Dumper's `IsoHeaderParser.cs`
/// `GetUnprotectedRegions`): big-endian region count, a reserved
/// `u32`, then `count` inclusive `(start, end)` sector pairs.
pub fn read_unprotected_regions(image: &[u8]) -> Result<Vec<(u32, u32)>, DiscCryptError> {
    if image.len() < 8 {
        return Err(DiscCryptError::TooSmall { len: image.len() });
    }
    let region_count = read_be_u32(image, 0);
    if region_count > MAX_REGIONS {
        return Err(DiscCryptError::TooManyRegions {
            count: region_count,
        });
    }
    // count + reserved word + (start,end) pairs.
    let needed = 8 + region_count as usize * 8;
    if image.len() < needed {
        return Err(DiscCryptError::RegionTableTruncated {
            needed,
            len: image.len(),
        });
    }
    let mut regions = Vec::with_capacity(region_count as usize);
    let mut off = 8;
    for _ in 0..region_count {
        let start = read_be_u32(image, off);
        let end = read_be_u32(image, off + 4);
        if start > end {
            return Err(DiscCryptError::BadRegionRange { start, end });
        }
        regions.push((start, end));
        off += 8;
    }
    Ok(regions)
}

/// Whether `sector` falls in any unprotected (plaintext) range.
fn is_unprotected(sector: u32, regions: &[(u32, u32)]) -> bool {
    regions.iter().any(|&(s, e)| s <= sector && sector <= e)
}

/// Decrypt an encrypted PS3 disc image with the user's `data1` (the
/// `.dkey`). Returns the plaintext image; unprotected sectors are
/// copied verbatim and protected sectors are CBC-decrypted.
///
/// Allocates the whole plaintext image; sized for tests and small
/// images. An installer-scale consumer streams
/// [`decrypt_disc_image_chunks`] instead -- a BD image does not fit
/// in memory twice.
pub fn decrypt_disc_image(image: &[u8], data1: &[u8; 16]) -> Result<Vec<u8>, DiscCryptError> {
    decrypt_disc_image_with_key(image, &decrypt_disc_key(data1))
}

/// Why streaming a decrypted disc image to its sink failed.
#[derive(Debug, thiserror::Error)]
pub enum DiscDecryptStreamError {
    /// The image itself is malformed (region table / alignment).
    #[error("disc decrypt: {0}")]
    Crypt(#[from] DiscCryptError),
    /// The sink refused a decrypted chunk.
    #[error("write decrypted sectors: {source}")]
    Sink {
        /// Underlying sink failure.
        #[source]
        source: std::io::Error,
    },
}

/// Sectors decrypted per scratch-buffer batch: 2048 sectors = 4 MiB,
/// the whole resident cost of decrypting an image of any size.
const BATCH_SECTORS: usize = 2048;

/// Lower-level form taking the already-derived content key, so a test
/// can drive it with an arbitrary key. Same residence caveat as
/// [`decrypt_disc_image`].
pub fn decrypt_disc_image_with_key(
    image: &[u8],
    key: &[u8; 16],
) -> Result<Vec<u8>, DiscCryptError> {
    let mut out = Vec::with_capacity(image.len());
    match decrypt_disc_image_chunks(image, key, |chunk| {
        out.extend_from_slice(chunk);
        Ok(())
    }) {
        Ok(()) => Ok(out),
        Err(DiscDecryptStreamError::Crypt(e)) => Err(e),
        Err(DiscDecryptStreamError::Sink { .. }) => {
            unreachable!("invariant: the Vec sink above never returns Err")
        }
    }
}

/// Stream-decrypt `image`, handing plaintext chunks to `sink` in image
/// order. Unprotected sectors pass through verbatim; protected sectors
/// are CBC-decrypted under `key` with the per-sector IV. Peak
/// residence is one 4 MiB scratch batch regardless of image size; the
/// chunks concatenate to exactly the image
/// [`decrypt_disc_image_with_key`] returns.
///
/// # Errors
///
/// [`DiscDecryptStreamError::Crypt`] for a malformed image (the same
/// refusals as the allocating form), [`DiscDecryptStreamError::Sink`]
/// when `sink` refuses a chunk.
pub fn decrypt_disc_image_chunks(
    image: &[u8],
    key: &[u8; 16],
    mut sink: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<(), DiscDecryptStreamError> {
    if !image.len().is_multiple_of(SECTOR) {
        return Err(DiscCryptError::ImageNotSectorAligned { len: image.len() }.into());
    }
    let regions = read_unprotected_regions(image)?;
    let total_sectors = image.len() / SECTOR;
    let mut batch = vec![0u8; BATCH_SECTORS * SECTOR];
    let mut sector = 0usize;
    while sector < total_sectors {
        let batch_sectors = BATCH_SECTORS.min(total_sectors - sector);
        let batch_bytes = batch_sectors * SECTOR;
        let image_base = sector * SECTOR;
        batch[..batch_bytes].copy_from_slice(&image[image_base..image_base + batch_bytes]);
        for i in 0..batch_sectors {
            let abs = sector + i;
            if is_unprotected(abs as u32, &regions) {
                continue;
            }
            let block = &mut batch[i * SECTOR..(i + 1) * SECTOR];
            let iv = sector_iv(abs as u64);
            // The alignment guard above makes every block exactly
            // SECTOR (2048) bytes -- a multiple of the 16-byte AES
            // block -- so NoPadding can never fail here.
            let _ = Aes128CbcDec::new(GenericArray::from_slice(key), GenericArray::from_slice(&iv))
                .decrypt_padded_mut::<NoPadding>(block)
                .expect("invariant: sector is 2048 bytes, a multiple of the 16-byte AES block");
        }
        sink(&batch[..batch_bytes]).map_err(|source| DiscDecryptStreamError::Sink { source })?;
        sector += batch_sectors;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/disc_crypt_tests.rs"]
mod tests;
