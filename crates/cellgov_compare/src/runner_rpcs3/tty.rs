//! TTY-log frame format and extractor.
//!
//! # TTY frame format
//!
//! The test writes `CGOV` (4 bytes) + big-endian u32 payload length +
//! raw payload bytes via `sys_tty_write`. The adapter locates the magic
//! tag in the log and slices regions from the payload in declaration
//! order.

use std::path::Path;

use crate::observation::NamedMemoryRegion;

use super::config::TtyRegion;
use super::error::Rpcs3Error;

/// Magic tag that precedes the big-endian u32 length and payload bytes.
pub const TTY_MAGIC: &[u8; 4] = b"CGOV";

/// TTY frame header: 4-byte magic + 4-byte length.
const TTY_HEADER_SIZE: usize = 8;

/// Scan a TTY log for the `CGOV` frame and slice the declared regions
/// from its payload in declaration order.
pub fn parse_tty_log(
    tty_path: &Path,
    regions: &[TtyRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(tty_path).map_err(Rpcs3Error::TtyRead)?;

    let magic_pos = data
        .windows(TTY_MAGIC.len())
        .position(|w| w == TTY_MAGIC.as_slice())
        .ok_or(Rpcs3Error::TtyMagicNotFound)?;

    // `magic_pos < data.len()` by find-position contract, so the
    // header_end add is bounded by `data.len() + TTY_HEADER_SIZE`,
    // well below `usize::MAX` for any real TTY log.
    let header_end = magic_pos + TTY_HEADER_SIZE;
    if header_end > data.len() {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: TTY_HEADER_SIZE as u64,
            actual: (data.len() - magic_pos) as u64,
        });
    }

    let len_bytes: [u8; 4] = data[magic_pos + 4..header_end].try_into().expect("4 bytes");
    let payload_len_u64 = u32::from_be_bytes(len_bytes) as u64;

    let payload_start = header_end;
    let payload_end_u64 = (payload_start as u64)
        .checked_add(payload_len_u64)
        .expect("payload_start + u32 length fits in u64");
    if payload_end_u64 > data.len() as u64 {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: payload_len_u64,
            actual: (data.len() - payload_start) as u64,
        });
    }
    let payload_end = payload_end_u64 as usize;
    let payload = &data[payload_start..payload_end];

    let mut total_needed: u64 = 0;
    for r in regions {
        total_needed =
            total_needed
                .checked_add(r.size)
                .ok_or_else(|| Rpcs3Error::TtyOffsetOverflow {
                    region_name: r.name.clone(),
                    size: r.size,
                })?;
    }
    if total_needed > payload_len_u64 {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: total_needed,
            actual: payload_len_u64,
        });
    }

    let mut offset: u64 = 0;
    let mut result = Vec::with_capacity(regions.len());
    for region in regions {
        let region_end =
            offset
                .checked_add(region.size)
                .ok_or_else(|| Rpcs3Error::TtyOffsetOverflow {
                    region_name: region.name.clone(),
                    size: region.size,
                })?;
        // total_needed <= payload_len_u64 <= u32::MAX, so the per-region
        // accumulator stays within usize on all supported hosts.
        let lo = offset as usize;
        let hi = region_end as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: payload[lo..hi].to_vec(),
        });
        offset = region_end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::config::Rpcs3Decoder;
    use super::super::observe_from_tty;
    use super::*;
    use crate::observation::ObservedOutcome;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Build a TTY log file with the framed protocol: CGOV + len + payload.
    fn write_tty_log(prefix: &[u8], payload: &[u8], suffix: &[u8]) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("tty_{n}.log"));
        let mut f = std::fs::File::create(&path).expect("create tty");
        f.write_all(prefix).expect("write prefix");
        f.write_all(TTY_MAGIC).expect("write magic");
        f.write_all(&(payload.len() as u32).to_be_bytes())
            .expect("write len");
        f.write_all(payload).expect("write payload");
        f.write_all(suffix).expect("write suffix");
        path
    }

    fn tty_region(name: &str, size: u64, addr: u64) -> TtyRegion {
        TtyRegion {
            name: name.into(),
            size,
            guest_addr: addr,
        }
    }

    #[test]
    fn parse_tty_log_extracts_single_region() {
        let payload = vec![0x00, 0x00, 0x00, 0x01, 0x13, 0x37, 0xBA, 0xAD];
        let path = write_tty_log(b"", &payload, b"");
        let regions = vec![tty_region("result", 8, 0x500000)];
        let parsed = parse_tty_log(&path, &regions).expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "result");
        assert_eq!(parsed[0].addr, 0x500000);
        assert_eq!(parsed[0].data, payload);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_extracts_multiple_regions() {
        let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
        let path = write_tty_log(b"", &payload, b"");
        let regions = vec![
            tty_region("status", 4, 0x500000),
            tty_region("value", 4, 0x500004),
        ];
        let parsed = parse_tty_log(&path, &regions).expect("parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(parsed[1].data, vec![0x11, 0x22, 0x33, 0x44]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_finds_tag_after_noise() {
        let noise = b"SPU Thread Group [0x1] started\nTest running...\n";
        let payload = vec![0x42, 0x00, 0x00, 0x00];
        let path = write_tty_log(noise, &payload, b"\nDone.\n");
        let regions = vec![tty_region("result", 4, 0x10000)];
        let parsed = parse_tty_log(&path, &regions).expect("parse");
        assert_eq!(parsed[0].data, vec![0x42, 0x00, 0x00, 0x00]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_no_magic_returns_error() {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("tty_nomag_{n}.log"));
        std::fs::write(&path, b"just some TTY noise\n").expect("write");
        let result = parse_tty_log(&path, &[tty_region("r", 4, 0)]);
        assert!(matches!(result, Err(Rpcs3Error::TtyMagicNotFound)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_truncated_payload_returns_error() {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("cellgov_rpcs3_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("tty_trunc_{n}.log"));
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(TTY_MAGIC).expect("magic");
        f.write_all(&100_u32.to_be_bytes()).expect("len");
        f.write_all(&[0u8; 10]).expect("short payload");
        drop(f);
        let result = parse_tty_log(&path, &[tty_region("r", 8, 0)]);
        assert!(matches!(result, Err(Rpcs3Error::TtyPayloadTooSmall { .. })));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_regions_exceed_payload_returns_error() {
        let payload = vec![0u8; 4];
        let path = write_tty_log(b"", &payload, b"");
        let regions = vec![tty_region("big", 8, 0)];
        let result = parse_tty_log(&path, &regions);
        assert!(matches!(result, Err(Rpcs3Error::TtyPayloadTooSmall { .. })));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_empty_regions_returns_empty_vec() {
        let payload = vec![0u8; 8];
        let path = write_tty_log(b"", &payload, b"");
        let parsed = parse_tty_log(&path, &[]).expect("parse");
        assert!(parsed.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tty_log_nonexistent_file_returns_error() {
        let result = parse_tty_log(Path::new("/nonexistent/tty.log"), &[]);
        assert!(matches!(result, Err(Rpcs3Error::TtyRead(_))));
    }

    #[test]
    fn observe_from_tty_builds_observation() {
        let payload = vec![0x00, 0x00, 0x00, 0x00, 0x13, 0x37, 0xBA, 0xAD];
        let path = write_tty_log(b"", &payload, b"");
        let regions = vec![tty_region("result", 8, 0)];
        let obs = observe_from_tty(&path, &regions, Rpcs3Decoder::Interpreter).expect("observe");
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.memory_regions.len(), 1);
        assert_eq!(obs.memory_regions[0].data, payload);
        assert_eq!(obs.metadata.runner, "rpcs3-interpreter");
        assert!(obs.state_hashes.is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn observe_from_tty_with_real_baseline() {
        let tty_path = Path::new("../../baselines/spu_fixed_value/rpcs3_interpreter.tty");
        if !tty_path.exists() {
            return;
        }
        let regions = vec![tty_region("result", 8, 0)];
        let obs = observe_from_tty(tty_path, &regions, Rpcs3Decoder::Interpreter).expect("observe");
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.memory_regions[0].data[4..8], [0x13, 0x37, 0xBA, 0xAD]);
    }
}
