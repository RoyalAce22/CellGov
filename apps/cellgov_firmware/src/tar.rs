//! USTAR TAR archive parser and extractor.

use std::path::{Path, PathBuf};

pub struct TarEntry {
    pub name: String,
    pub data: Vec<u8>,
}

fn octal_to_u64(s: &[u8]) -> u64 {
    let s = std::str::from_utf8(s).unwrap_or("0");
    let s = s.trim_end_matches('\0').trim();
    u64::from_str_radix(s, 8).unwrap_or(0)
}

pub fn parse(data: &[u8]) -> Vec<TarEntry> {
    let mut entries = Vec::new();
    let mut offset = 0usize;

    while offset + 512 <= data.len() {
        let header = &data[offset..offset + 512];

        if header.iter().all(|&b| b == 0) {
            break;
        }

        let name_raw = &header[0..100];
        let name_end = name_raw.iter().position(|&b| b == 0).unwrap_or(100);
        let name_str = std::str::from_utf8(&name_raw[..name_end]).unwrap_or("");

        let prefix_raw = &header[0x155..0x155 + 155];
        let prefix_end = prefix_raw.iter().position(|&b| b == 0).unwrap_or(155);
        let prefix_str = std::str::from_utf8(&prefix_raw[..prefix_end]).unwrap_or("");

        let full_name = if prefix_str.is_empty() {
            name_str.to_string()
        } else {
            format!("{prefix_str}/{name_str}")
        };

        let size = octal_to_u64(&header[0x7C..0x7C + 12]) as usize;
        let filetype = header[0x9C];

        offset += 512;

        if (filetype == b'0' || filetype == 0) && size > 0 && offset + size <= data.len() {
            entries.push(TarEntry {
                name: full_name,
                data: data[offset..offset + size].to_vec(),
            });
        }

        let aligned = (size + 511) & !511;
        offset += aligned;
    }

    entries
}

pub fn extract_to_disk(entries: &[TarEntry], base: &Path) -> usize {
    let mut count = 0;
    for entry in entries {
        if entry.name.is_empty() || entry.data.is_empty() {
            continue;
        }
        let clean = entry.name.trim_start_matches('/');
        // Strip leading "000/" prefix from PUP inner TARs.
        let clean = clean.strip_prefix("000/").unwrap_or(clean);
        if clean.is_empty() {
            continue;
        }
        let dest: PathBuf = base.join(clean);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&dest, &entry.data).is_ok() {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_returns_empty() {
        assert!(parse(&[]).is_empty());
    }

    #[test]
    fn parse_all_zeros_returns_empty() {
        assert!(parse(&[0u8; 1024]).is_empty());
    }

    #[test]
    fn parse_minimal_tar_entry() {
        let mut block = [0u8; 1024];
        // name: "hello.txt"
        block[0..9].copy_from_slice(b"hello.txt");
        // size: "5" in octal at offset 0x7C
        block[0x7C..0x7C + 2].copy_from_slice(b"5\0");
        // filetype: '0' (regular file)
        block[0x9C] = b'0';
        // magic: "ustar" at offset 0x101
        block[0x101..0x106].copy_from_slice(b"ustar");
        // data: "hello" at offset 512
        block[512..517].copy_from_slice(b"hello");

        let entries = parse(&block);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "hello.txt");
        assert_eq!(entries[0].data, b"hello");
    }
}
