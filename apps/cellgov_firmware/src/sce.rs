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
pub struct SceContainerHeader {
    pub magic: u32,
    pub header_version: u32,
    pub revision_flags: u16,
    pub category: u16,
    pub metadata_offset: u32,
    pub header_size: u64,
    pub encrypted_payload_size: u64,
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct MetadataKeyEnvelope {
    pub aes_key: [u8; 16],
    pub aes_key_padding: [u8; 16],
    pub aes_iv: [u8; 16],
    pub aes_iv_padding: [u8; 16],
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct EncryptedMetadataDirectory {
    pub signed_region_length: u64,
    pub reserved_a: u32,
    pub section_count: u32,
    pub key_count: u32,
    pub auxiliary_header_size: u32,
    pub reserved_b: u32,
    pub reserved_c: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct EncryptedSectionDescriptor {
    pub payload_offset: u64,
    pub payload_size: u64,
    pub section_kind: u32,
    pub program_segment_index: u32,
    pub sha1_hashed: u32,
    pub sha1_slot: u32,
    pub encryption_kind: u32,
    pub key_slot: u32,
    pub iv_slot: u32,
    pub compression_kind: u32,
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

fn parse_sce_header(data: &[u8]) -> Result<SceContainerHeader, String> {
    if data.len() < 0x20 {
        return Err("data too small for SCE header".into());
    }
    let magic = read_be_u32(data, 0);
    if magic != 0x53434500 {
        return Err(format!("bad SCE magic: 0x{magic:08x}"));
    }
    Ok(SceContainerHeader {
        magic,
        header_version: read_be_u32(data, 4),
        revision_flags: read_be_u16(data, 8),
        category: read_be_u16(data, 10),
        metadata_offset: read_be_u32(data, 12),
        header_size: read_be_u64(data, 16),
        encrypted_payload_size: read_be_u64(data, 24),
    })
}

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

pub fn decrypt_package(data: &[u8]) -> Result<Vec<u8>, String> {
    decrypt_sce(data, &SCEPKG_ERK, &SCEPKG_RIV)
}

pub fn decrypt_self_to_elf(data: &[u8]) -> Result<Vec<u8>, String> {
    let hdr = parse_sce_header(data)?;
    let revision = hdr.revision_flags & 0x7FFF;
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

    let key_envelope_offset = hdr.metadata_offset as usize + 0x20;
    if key_envelope_offset + 0x40 > data.len() {
        return Err("SCE file truncated at metadata info".into());
    }

    let mut key_envelope_buf = [0u8; 0x40];
    key_envelope_buf.copy_from_slice(&data[key_envelope_offset..key_envelope_offset + 0x40]);

    // Debug SELFs ship the key envelope in cleartext; retail encrypts with AES-256-CBC.
    let is_debug = (hdr.revision_flags & 0x8000) != 0;
    if !is_debug {
        let decryptor = Aes256CbcDec::new(
            aes::cipher::generic_array::GenericArray::from_slice(erk),
            aes::cipher::generic_array::GenericArray::from_slice(riv),
        );
        decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut key_envelope_buf)
            .map_err(|e| format!("AES-256-CBC decrypt failed: {e}"))?;
    }

    let aes_key: [u8; 16] = key_envelope_buf[0..16].try_into().unwrap();
    let aes_iv: [u8; 16] = key_envelope_buf[0x20..0x30].try_into().unwrap();

    // The two 16-byte padding regions must decrypt to zero; non-zero means the ERK/RIV
    // does not match this SELF revision.
    if !is_debug
        && (key_envelope_buf[0x10..0x20].iter().any(|&b| b != 0)
            || key_envelope_buf[0x30..0x40].iter().any(|&b| b != 0))
    {
        return Err("MetadataKeyEnvelope padding validation failed (wrong key?)".into());
    }

    let directory_offset = key_envelope_offset + 0x40;
    let directory_end = hdr.header_size as usize;
    if directory_end > data.len() || directory_offset >= directory_end {
        return Err("SCE file truncated at metadata headers".into());
    }
    let mut directory_buf = data[directory_offset..directory_end].to_vec();

    let mut ctr_cipher = Aes128Ctr::new(
        aes::cipher::generic_array::GenericArray::from_slice(&aes_key),
        aes::cipher::generic_array::GenericArray::from_slice(&aes_iv),
    );
    ctr_cipher.seek(0u64);
    ctr_cipher.apply_keystream(&mut directory_buf);

    if directory_buf.len() < 0x20 {
        return Err("decrypted metadata too small for header".into());
    }
    let section_count = read_be_u32(&directory_buf, 0x0C) as usize;
    let key_count = read_be_u32(&directory_buf, 0x10) as usize;

    let sections_start = 0x20usize;
    let keys_start = sections_start + section_count * 0x30;
    let keys_end = keys_start + key_count * 0x10;

    if keys_end > directory_buf.len() {
        return Err(format!(
            "metadata headers truncated: need {} bytes, have {}",
            keys_end,
            directory_buf.len()
        ));
    }

    let data_keys = &directory_buf[keys_start..keys_end];

    let mut sections: Vec<Vec<u8>> = Vec::new();

    for i in 0..section_count {
        let off = sections_start + i * 0x30;
        let sec = EncryptedSectionDescriptor {
            payload_offset: read_be_u64(&directory_buf, off),
            payload_size: read_be_u64(&directory_buf, off + 8),
            section_kind: read_be_u32(&directory_buf, off + 0x10),
            program_segment_index: read_be_u32(&directory_buf, off + 0x14),
            sha1_hashed: read_be_u32(&directory_buf, off + 0x18),
            sha1_slot: read_be_u32(&directory_buf, off + 0x1C),
            encryption_kind: read_be_u32(&directory_buf, off + 0x20),
            key_slot: read_be_u32(&directory_buf, off + 0x24),
            iv_slot: read_be_u32(&directory_buf, off + 0x28),
            compression_kind: read_be_u32(&directory_buf, off + 0x2C),
        };

        let sec_start = sec.payload_offset as usize;
        let sec_end = sec_start + sec.payload_size as usize;
        if sec_end > data.len() {
            return Err(format!("section {i} extends past file end"));
        }

        let mut sec_data = data[sec_start..sec_end].to_vec();

        if sec.encryption_kind == 3 {
            let k_off = sec.key_slot as usize * 0x10;
            let iv_off = sec.iv_slot as usize * 0x10;
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

        if sec.compression_kind == 2 {
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
        assert_eq!(hdr.magic, 0x53434500);
        assert_eq!(hdr.header_size, 256);
    }

    #[test]
    fn decrypt_package_rejects_truncated() {
        assert!(decrypt_package(&[0u8; 8]).is_err());
    }
}
