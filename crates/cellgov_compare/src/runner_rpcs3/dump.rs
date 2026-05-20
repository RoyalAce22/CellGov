//! Memory-dump region extractor.

use std::path::Path;

use crate::observation::NamedMemoryRegion;

use super::config::DumpRegion;
use super::error::Rpcs3Error;

/// Read a memory dump file and extract the declared regions.
pub fn parse_dump(
    dump_path: &Path,
    regions: &[DumpRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(dump_path).map_err(Rpcs3Error::DumpRead)?;
    let data_len = data.len() as u64;
    let mut result = Vec::with_capacity(regions.len());

    for region in regions {
        let end = region.offset.checked_add(region.size).ok_or_else(|| {
            Rpcs3Error::DumpOffsetOverflow {
                region_name: region.name.clone(),
                offset: region.offset,
                size: region.size,
            }
        })?;
        if end > data_len {
            return Err(Rpcs3Error::DumpTooSmall {
                region_name: region.name.clone(),
                guest_addr: region.guest_addr,
                expected: end,
                actual: data_len,
            });
        }
        let start = region.offset as usize;
        let end_usz = end as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: data[start..end_usz].to_vec(),
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn write_temp_dump(data: &[u8]) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("dump_{n}.bin"));
        let mut f = std::fs::File::create(&path).expect("create dump");
        f.write_all(data).expect("write dump");
        path
    }

    #[test]
    fn parse_dump_extracts_single_region() {
        let data = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
        let path = write_temp_dump(&data);
        let regions = vec![DumpRegion {
            name: "result".into(),
            offset: 4,
            size: 4,
            guest_addr: 0x10000,
        }];
        let parsed = parse_dump(&path, &regions).expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "result");
        assert_eq!(parsed[0].addr, 0x10000);
        assert_eq!(parsed[0].data, vec![0x11, 0x22, 0x33, 0x44]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_dump_extracts_multiple_regions() {
        let data = vec![0; 32];
        let path = write_temp_dump(&data);
        let regions = vec![
            DumpRegion {
                name: "a".into(),
                offset: 0,
                size: 8,
                guest_addr: 0x1000,
            },
            DumpRegion {
                name: "b".into(),
                offset: 16,
                size: 8,
                guest_addr: 0x2000,
            },
        ];
        let parsed = parse_dump(&path, &regions).expect("parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "a");
        assert_eq!(parsed[1].name, "b");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_dump_rejects_region_past_end() {
        let data = vec![0; 8];
        let path = write_temp_dump(&data);
        let regions = vec![DumpRegion {
            name: "oob".into(),
            offset: 4,
            size: 8, // extends past end
            guest_addr: 0x1000,
        }];
        let result = parse_dump(&path, &regions);
        match result.unwrap_err() {
            Rpcs3Error::DumpTooSmall {
                region_name,
                guest_addr,
                expected,
                actual,
            } => {
                assert_eq!(region_name, "oob");
                assert_eq!(guest_addr, 0x1000);
                assert_eq!(expected, 12);
                assert_eq!(actual, 8);
            }
            other => panic!("expected DumpTooSmall, got {other:?}"),
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_dump_rejects_offset_size_overflow() {
        let data = vec![0; 8];
        let path = write_temp_dump(&data);
        let regions = vec![DumpRegion {
            name: "overflow".into(),
            offset: u64::MAX - 4,
            size: 100,
            guest_addr: 0x1000,
        }];
        match parse_dump(&path, &regions).unwrap_err() {
            Rpcs3Error::DumpOffsetOverflow {
                region_name,
                offset,
                size,
            } => {
                assert_eq!(region_name, "overflow");
                assert_eq!(offset, u64::MAX - 4);
                assert_eq!(size, 100);
            }
            other => panic!("expected DumpOffsetOverflow, got {other:?}"),
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_dump_empty_regions_returns_empty_vec() {
        let data = vec![0; 8];
        let path = write_temp_dump(&data);
        let parsed = parse_dump(&path, &[]).expect("parse");
        assert!(parsed.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_dump_nonexistent_file_returns_error() {
        let result = parse_dump(Path::new("/nonexistent/dump.bin"), &[]);
        assert!(result.is_err());
    }
}
