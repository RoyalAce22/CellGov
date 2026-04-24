//! SCE/SELF package decrypter for PS3 firmware and game binaries.
//!
//! All SCE/SELF headers are big-endian. The ELF produced by
//! [`decrypt_self_to_elf`] is plaintext only: segment signatures and the
//! outer SCE signature are dropped, so callers must not re-sign or feed the
//! output to anything that verifies signatures.

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
    decrypt_sce(data, &SCEPKG_ERK, &SCEPKG_RIV)
}

pub fn decrypt_self_to_elf(data: &[u8]) -> Result<Vec<u8>, String> {
    let hdr = parse_sce_header(data)?;
    let revision = hdr.se_flags & 0x7FFF;
    let key = crate::crypto::app_key_for_revision(revision)
        .ok_or_else(|| format!("no APP key for SELF revision 0x{revision:04x}"))?;

    if data.len() < 0x40 {
        return Err("SELF too short for extended header".into());
    }
    let ehdr_offset = read_be_u64(data, 0x30) as usize;
    let phdr_offset = read_be_u64(data, 0x38) as usize;

    if ehdr_offset + 0x40 > data.len() {
        return Err("SELF ELF header offset out of range".into());
    }
    let e_phnum = read_be_u16(data, ehdr_offset + 0x38) as usize;
    let e_phentsize = read_be_u16(data, ehdr_offset + 0x36) as usize;
    if phdr_offset + e_phnum * e_phentsize > data.len() {
        return Err("SELF program headers out of range".into());
    }

    let sections = decrypt_sce_sections(data, &key.erk, &key.riv)?;

    let mut elf_size: usize = 0x40 + e_phnum * e_phentsize;
    for i in 0..e_phnum {
        let ph_off = phdr_offset + i * e_phentsize;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        let end = p_offset + p_filesz;
        if end > elf_size {
            elf_size = end;
        }
    }

    let mut elf = vec![0u8; elf_size];
    elf[..0x40].copy_from_slice(&data[ehdr_offset..ehdr_offset + 0x40]);
    let phdr_dst = 0x40usize;
    elf[phdr_dst..phdr_dst + e_phnum * e_phentsize]
        .copy_from_slice(&data[phdr_offset..phdr_offset + e_phnum * e_phentsize]);
    elf[0x20..0x28].copy_from_slice(&(phdr_dst as u64).to_be_bytes());

    for (sec_idx, sec_data) in sections.iter().enumerate() {
        if sec_idx >= e_phnum {
            break;
        }
        let ph_off = phdr_offset + sec_idx * e_phentsize;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        let copy_len = sec_data.len().min(p_filesz);
        if p_offset + copy_len <= elf.len() && !sec_data.is_empty() {
            elf[p_offset..p_offset + copy_len].copy_from_slice(&sec_data[..copy_len]);
        }
    }

    let magic = u32::from_be_bytes([elf[0], elf[1], elf[2], elf[3]]);
    if magic != 0x7F454C46 {
        return Err(format!("reconstructed ELF has bad magic: 0x{magic:08x}"));
    }

    Ok(elf)
}

fn decrypt_sce(data: &[u8], erk: &[u8; 0x20], riv: &[u8; 0x10]) -> Result<Vec<u8>, String> {
    let sections = decrypt_sce_sections(data, erk, riv)?;

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

    for (i, s) in sections.iter().enumerate() {
        if s.len() >= 0x107 && &s[0x101..0x106] == b"ustar" {
            if std::env::var("CELLGOV_FW_DEBUG").is_ok() {
                eprintln!("    -> using section[{i}] (ustar TAR)");
            }
            return Ok(sections.into_iter().nth(i).unwrap());
        }
    }

    if let Some(largest) = sections.into_iter().max_by_key(|s| s.len()) {
        Ok(largest)
    } else {
        Err("no usable section found in decrypted package".into())
    }
}

fn decrypt_sce_sections(
    data: &[u8],
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<Vec<Vec<u8>>, String> {
    let hdr = parse_sce_header(data)?;

    let meta_offset = hdr.se_meta as usize + 0x20;
    if meta_offset + 0x40 > data.len() {
        return Err("SCE file truncated at metadata info".into());
    }

    let mut meta_info_buf = [0u8; 0x40];
    meta_info_buf.copy_from_slice(&data[meta_offset..meta_offset + 0x40]);

    // Debug SELFs ship MetadataInfo in cleartext; retail encrypts with AES-256-CBC.
    let is_debug = (hdr.se_flags & 0x8000) != 0;
    if !is_debug {
        let decryptor = Aes256CbcDec::new(
            aes::cipher::generic_array::GenericArray::from_slice(erk),
            aes::cipher::generic_array::GenericArray::from_slice(riv),
        );
        decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut meta_info_buf)
            .map_err(|e| format!("AES-256-CBC decrypt failed: {e}"))?;
    }

    let meta_key: [u8; 16] = meta_info_buf[0..16].try_into().unwrap();
    let meta_iv: [u8; 16] = meta_info_buf[0x20..0x30].try_into().unwrap();

    // The two 16-byte pad regions must decrypt to zero; non-zero means the ERK/RIV
    // does not match this SELF revision.
    if !is_debug
        && (meta_info_buf[0x10..0x20].iter().any(|&b| b != 0)
            || meta_info_buf[0x30..0x40].iter().any(|&b| b != 0))
    {
        return Err("MetadataInfo padding validation failed (wrong key?)".into());
    }

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

    Ok(sections)
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
        data[16..24].copy_from_slice(&256u64.to_be_bytes());
        let hdr = parse_sce_header(&data).unwrap();
        assert_eq!(hdr.se_magic, 0x53434500);
        assert_eq!(hdr.se_hsize, 256);
    }

    #[test]
    fn decrypt_package_rejects_truncated() {
        assert!(decrypt_package(&[0u8; 8]).is_err());
    }
}
