//! SCE/SELF package decrypter for PS3 firmware.

use aes::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, StreamCipherSeek};

use crate::crypto::{SCEPKG_ERK, SCEPKG_RIV};

#[derive(Debug)]
#[repr(C)]
pub struct SceHeader {
    pub se_magic: u32,
    pub se_hver: u32,
    pub se_flags: u16,
    pub se_type: u16,
    pub se_meta: u32,
    pub se_hsize: u64,
    pub se_esize: u64,
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct MetadataInfo {
    pub key: [u8; 16],
    pub key_pad: [u8; 16],
    pub iv: [u8; 16],
    pub iv_pad: [u8; 16],
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct MetadataHeader {
    pub signature_input_length: u64,
    pub unknown1: u32,
    pub section_count: u32,
    pub key_count: u32,
    pub opt_header_size: u32,
    pub unknown2: u32,
    pub unknown3: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct MetadataSectionHeader {
    pub data_offset: u64,
    pub data_size: u64,
    pub section_type: u32,
    pub program_idx: u32,
    pub hashed: u32,
    pub sha1_idx: u32,
    pub encrypted: u32,
    pub key_idx: u32,
    pub iv_idx: u32,
    pub compressed: u32,
}

fn read_be_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_be_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_be_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn parse_sce_header(data: &[u8]) -> Result<SceHeader, String> {
    if data.len() < 0x20 {
        return Err("data too small for SCE header".into());
    }
    let magic = read_be_u32(data, 0);
    if magic != 0x53434500 {
        return Err(format!("bad SCE magic: 0x{magic:08x}"));
    }
    Ok(SceHeader {
        se_magic: magic,
        se_hver: read_be_u32(data, 4),
        se_flags: read_be_u16(data, 8),
        se_type: read_be_u16(data, 10),
        se_meta: read_be_u32(data, 12),
        se_hsize: read_be_u64(data, 16),
        se_esize: read_be_u64(data, 24),
    })
}

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

pub fn decrypt_package(data: &[u8]) -> Result<Vec<u8>, String> {
    let hdr = parse_sce_header(data)?;

    let meta_offset = hdr.se_meta as usize + 0x20;
    if meta_offset + 0x40 > data.len() {
        return Err("SCE file truncated at metadata info".into());
    }

    // Step 1: Decrypt MetadataInfo (AES-256-CBC)
    let mut meta_info_buf = [0u8; 0x40];
    meta_info_buf.copy_from_slice(&data[meta_offset..meta_offset + 0x40]);

    let is_debug = (hdr.se_flags & 0x8000) != 0;
    if !is_debug {
        let decryptor = Aes256CbcDec::new(
            aes::cipher::generic_array::GenericArray::from_slice(&SCEPKG_ERK),
            aes::cipher::generic_array::GenericArray::from_slice(&SCEPKG_RIV),
        );
        decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut meta_info_buf)
            .map_err(|e| format!("AES-256-CBC decrypt failed: {e}"))?;
    }

    let meta_key: [u8; 16] = meta_info_buf[0..16].try_into().unwrap();
    let meta_iv: [u8; 16] = meta_info_buf[0x20..0x30].try_into().unwrap();

    if !is_debug
        && (meta_info_buf[0x10..0x20].iter().any(|&b| b != 0)
            || meta_info_buf[0x30..0x40].iter().any(|&b| b != 0))
    {
        return Err("MetadataInfo padding validation failed (wrong key?)".into());
    }

    // Step 2: Decrypt metadata headers (AES-128-CTR)
    let headers_offset = meta_offset + 0x40;
    let headers_end = hdr.se_hsize as usize;
    if headers_end > data.len() || headers_offset >= headers_end {
        return Err("SCE file truncated at metadata headers".into());
    }
    let mut headers_buf = data[headers_offset..headers_end].to_vec();

    let mut ctr_cipher = Aes128Ctr::new(
        aes::cipher::generic_array::GenericArray::from_slice(&meta_key),
        aes::cipher::generic_array::GenericArray::from_slice(&meta_iv),
    );
    ctr_cipher.seek(0u64);
    ctr_cipher.apply_keystream(&mut headers_buf);

    // Step 3: Parse decrypted metadata headers
    if headers_buf.len() < 0x20 {
        return Err("decrypted metadata too small for header".into());
    }
    let section_count = read_be_u32(&headers_buf, 0x0C) as usize;
    let key_count = read_be_u32(&headers_buf, 0x10) as usize;

    let sections_start = 0x20usize;
    let keys_start = sections_start + section_count * 0x30;
    let keys_end = keys_start + key_count * 0x10;

    if keys_end > headers_buf.len() {
        return Err(format!(
            "metadata headers truncated: need {} bytes, have {}",
            keys_end,
            headers_buf.len()
        ));
    }

    let data_keys = &headers_buf[keys_start..keys_end];

    // Step 4: Decrypt each section
    let mut sections: Vec<Vec<u8>> = Vec::new();

    for i in 0..section_count {
        let off = sections_start + i * 0x30;
        let sec = MetadataSectionHeader {
            data_offset: read_be_u64(&headers_buf, off),
            data_size: read_be_u64(&headers_buf, off + 8),
            section_type: read_be_u32(&headers_buf, off + 0x10),
            program_idx: read_be_u32(&headers_buf, off + 0x14),
            hashed: read_be_u32(&headers_buf, off + 0x18),
            sha1_idx: read_be_u32(&headers_buf, off + 0x1C),
            encrypted: read_be_u32(&headers_buf, off + 0x20),
            key_idx: read_be_u32(&headers_buf, off + 0x24),
            iv_idx: read_be_u32(&headers_buf, off + 0x28),
            compressed: read_be_u32(&headers_buf, off + 0x2C),
        };

        let sec_start = sec.data_offset as usize;
        let sec_end = sec_start + sec.data_size as usize;
        if sec_end > data.len() {
            return Err(format!("section {i} extends past file end"));
        }

        let mut sec_data = data[sec_start..sec_end].to_vec();

        if sec.encrypted == 3 {
            let k_off = sec.key_idx as usize * 0x10;
            let iv_off = sec.iv_idx as usize * 0x10;
            if k_off + 0x10 > data_keys.len() || iv_off + 0x10 > data_keys.len() {
                return Err(format!("section {i} key/iv index out of range"));
            }
            let sec_key: [u8; 16] = data_keys[k_off..k_off + 0x10].try_into().unwrap();
            let sec_iv: [u8; 16] = data_keys[iv_off..iv_off + 0x10].try_into().unwrap();

            let mut sec_cipher = Aes128Ctr::new(
                aes::cipher::generic_array::GenericArray::from_slice(&sec_key),
                aes::cipher::generic_array::GenericArray::from_slice(&sec_iv),
            );
            sec_cipher.seek(0u64);
            sec_cipher.apply_keystream(&mut sec_data);
        }

        if sec.compressed == 2 {
            use flate2::read::ZlibDecoder;
            use std::io::Read;
            let mut decoder = ZlibDecoder::new(sec_data.as_slice());
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| format!("zlib decompress failed for section {i}: {e}"))?;
            sec_data = decompressed;
        }

        sections.push(sec_data);
    }

    if std::env::var("CELLGOV_FW_DEBUG").is_ok() {
        for (i, s) in sections.iter().enumerate() {
            let magic = if s.len() >= 4 {
                format!("{:02x}{:02x}{:02x}{:02x}", s[0], s[1], s[2], s[3])
            } else {
                "??".to_string()
            };
            eprintln!("    section[{i}]: {} bytes, magic={magic}", s.len());
        }
    }

    // Find the section that is a valid TAR (starts with a non-null
    // filename and has "ustar" at offset 0x101).
    for (i, s) in sections.iter().enumerate() {
        if s.len() >= 0x107 && &s[0x101..0x106] == b"ustar" {
            if std::env::var("CELLGOV_FW_DEBUG").is_ok() {
                eprintln!("    -> using section[{i}] (ustar TAR)");
            }
            return Ok(sections.into_iter().nth(i).unwrap());
        }
    }

    // Fallback: largest section
    if let Some(largest) = sections.into_iter().max_by_key(|s| s.len()) {
        Ok(largest)
    } else {
        Err("no usable section found in decrypted package".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sce_header_rejects_short() {
        assert!(parse_sce_header(&[0u8; 16]).is_err());
    }

    #[test]
    fn parse_sce_header_rejects_bad_magic() {
        let mut data = [0u8; 0x20];
        data[0..4].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
        assert!(parse_sce_header(&data)
            .unwrap_err()
            .contains("bad SCE magic"));
    }

    #[test]
    fn parse_sce_header_accepts_valid() {
        let mut data = [0u8; 0x20];
        data[0..4].copy_from_slice(&0x53434500u32.to_be_bytes());
        data[16..24].copy_from_slice(&256u64.to_be_bytes()); // se_hsize
        let hdr = parse_sce_header(&data).unwrap();
        assert_eq!(hdr.se_magic, 0x53434500);
        assert_eq!(hdr.se_hsize, 256);
    }

    #[test]
    fn decrypt_package_rejects_truncated() {
        assert!(decrypt_package(&[0u8; 8]).is_err());
    }
}
