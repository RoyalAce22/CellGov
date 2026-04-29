//! RPCS3 runner adapter: invokes the patched RPCS3 binary headless,
//! then extracts the microtest result from either a binary memory dump
//! or the RPCS3 TTY log, and packs it into an `Observation`.
//!
//! # TTY frame format
//!
//! The test writes `CGOV` (4 bytes) + big-endian u32 payload length +
//! raw payload bytes via `sys_tty_write`. The adapter locates the magic
//! tag in the log and slices regions from the payload in declaration
//! order. `DumpFile` extraction is the alternate path when TTY capture
//! is unavailable.

use crate::observation::{NamedMemoryRegion, Observation, ObservationMetadata, ObservedOutcome};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Magic tag that precedes the big-endian u32 length and payload bytes.
pub const TTY_MAGIC: &[u8; 4] = b"CGOV";

/// TTY frame header: 4-byte magic + 4-byte length.
const TTY_HEADER_SIZE: usize = 8;

/// RPCS3 installation and global settings.
#[derive(Debug, Clone)]
pub struct Rpcs3Config {
    /// Path to the rpcs3 executable.
    pub executable: PathBuf,
    /// Decoder mode for the test run.
    pub decoder: Rpcs3Decoder,
}

/// Which RPCS3 decoder combination to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpcs3Decoder {
    /// PPU Interpreter + SPU Interpreter.
    Interpreter,
    /// PPU LLVM + SPU LLVM.
    Llvm,
}

/// How to extract the result buffer from RPCS3 after a test run.
#[derive(Debug, Clone)]
pub enum ExtractionMethod {
    /// Read regions at byte offsets within a binary memory dump file.
    DumpFile {
        /// Path to the dump file.
        path: PathBuf,
        /// Regions to extract from the dump.
        regions: Vec<DumpRegion>,
    },
    /// Scan the TTY log for the `CGOV` frame and slice regions from its
    /// payload in declaration order.
    TtyLog {
        /// Path to RPCS3's TTY.log.
        path: PathBuf,
        /// Regions to extract from the CGOV payload.
        regions: Vec<TtyRegion>,
    },
}

/// Per-test configuration for an RPCS3 run.
#[derive(Debug, Clone)]
pub struct Rpcs3TestConfig {
    /// Path to the ELF binary to execute.
    pub binary: PathBuf,
    /// Wall-clock timeout for the RPCS3 process.
    pub timeout: Duration,
    /// How to extract the result buffer after the run.
    pub extraction: ExtractionMethod,
}

/// A region within a binary memory dump file.
#[derive(Debug, Clone)]
pub struct DumpRegion {
    /// Region name.
    pub name: String,
    /// Byte offset within the dump file.
    pub offset: u64,
    /// Number of bytes to read.
    pub size: u64,
    /// Guest address to report in the observation.
    pub guest_addr: u64,
}

/// A region to extract from the TTY payload; regions are packed
/// contiguously in declaration order.
#[derive(Debug, Clone)]
pub struct TtyRegion {
    /// Region name.
    pub name: String,
    /// Number of bytes for this region within the payload.
    pub size: u64,
    /// Guest address to report in the observation.
    pub guest_addr: u64,
}

/// Why an RPCS3 run failed.
#[derive(Debug)]
pub enum Rpcs3Error {
    /// The RPCS3 process could not be started.
    Launch(io::Error),
    /// The RPCS3 process exceeded the wall-clock timeout.
    Timeout,
    /// The memory dump file could not be read.
    DumpRead(io::Error),
    /// The dump file is too small for the declared regions.
    DumpTooSmall {
        /// Minimum size required by declared regions.
        expected: u64,
        /// Actual file size.
        actual: u64,
    },
    /// The TTY log file could not be read.
    TtyRead(io::Error),
    /// The TTY log does not contain the expected magic tag.
    TtyMagicNotFound,
    /// The TTY payload is shorter than declared regions require.
    TtyPayloadTooSmall {
        /// Minimum payload size required by declared regions.
        expected: u64,
        /// Actual payload size.
        actual: u64,
    },
}

/// Invoke RPCS3 headless, then extract regions via the configured method.
pub fn observe(config: &Rpcs3Config, test: &Rpcs3TestConfig) -> Result<Observation, Rpcs3Error> {
    let outcome = invoke(config, test)?;
    let memory_regions = match &test.extraction {
        ExtractionMethod::DumpFile { path, regions } => parse_dump(path, regions)?,
        ExtractionMethod::TtyLog { path, regions } => parse_tty_log(path, regions)?,
    };

    Ok(Observation {
        outcome,
        memory_regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: format!("rpcs3-{:?}", config.decoder).to_lowercase(),
            steps: None,
        },
        // Region-extraction adapter does not surface raw TTY bytes
        // beyond the magic-tagged payload it parses. Step-2 ps3autotests
        // path will read TTY directly via its own helper.
        tty_log: Vec::new(),
    })
}

/// Build an observation from a saved TTY log without invoking RPCS3;
/// the outcome is forced to `Completed`.
pub fn observe_from_tty(
    tty_path: &Path,
    regions: &[TtyRegion],
    decoder: Rpcs3Decoder,
) -> Result<Observation, Rpcs3Error> {
    let memory_regions = parse_tty_log(tty_path, regions)?;
    Ok(Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: format!("rpcs3-{:?}", decoder).to_lowercase(),
            steps: None,
        },
        tty_log: Vec::new(),
    })
}

/// Launch RPCS3 and wait for exit or timeout. Returns the mapped outcome.
fn invoke(config: &Rpcs3Config, test: &Rpcs3TestConfig) -> Result<ObservedOutcome, Rpcs3Error> {
    let mut child = Command::new(&config.executable)
        .arg("--headless")
        .arg(&test.binary)
        .spawn()
        .map_err(Rpcs3Error::Launch)?;

    let deadline = std::time::Instant::now() + test.timeout;
    let exit_status = loop {
        match child.try_wait().map_err(Rpcs3Error::Launch)? {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(ObservedOutcome::Timeout);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let outcome = if exit_status.success() {
        ObservedOutcome::Completed
    } else {
        ObservedOutcome::Fault
    };
    Ok(outcome)
}

/// Read a memory dump file and extract the declared regions.
pub fn parse_dump(
    dump_path: &Path,
    regions: &[DumpRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(dump_path).map_err(Rpcs3Error::DumpRead)?;
    let mut result = Vec::with_capacity(regions.len());

    for region in regions {
        let start = region.offset as usize;
        let end = start + region.size as usize;
        if end > data.len() {
            return Err(Rpcs3Error::DumpTooSmall {
                expected: end as u64,
                actual: data.len() as u64,
            });
        }
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: data[start..end].to_vec(),
        });
    }

    Ok(result)
}

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

    let header_end = magic_pos + TTY_HEADER_SIZE;
    if header_end > data.len() {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: TTY_HEADER_SIZE as u64,
            actual: (data.len() - magic_pos) as u64,
        });
    }

    let len_bytes: [u8; 4] = data[magic_pos + 4..header_end].try_into().expect("4 bytes");
    let payload_len = u32::from_be_bytes(len_bytes) as usize;

    let payload_start = header_end;
    let payload_end = payload_start + payload_len;
    if payload_end > data.len() {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: payload_len as u64,
            actual: (data.len() - payload_start) as u64,
        });
    }

    let payload = &data[payload_start..payload_end];

    let total_needed: u64 = regions.iter().map(|r| r.size).sum();
    if total_needed > payload_len as u64 {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: total_needed,
            actual: payload_len as u64,
        });
    }

    let mut offset = 0usize;
    let mut result = Vec::with_capacity(regions.len());
    for region in regions {
        let size = region.size as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: payload[offset..offset + size].to_vec(),
        });
        offset += size;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
        assert!(result.is_err());
        match result.unwrap_err() {
            Rpcs3Error::DumpTooSmall { expected, actual } => {
                assert_eq!(expected, 12);
                assert_eq!(actual, 8);
            }
            other => panic!("expected DumpTooSmall, got {other:?}"),
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

    #[test]
    fn decoder_format_in_metadata() {
        let name = format!("rpcs3-{:?}", Rpcs3Decoder::Interpreter).to_lowercase();
        assert_eq!(name, "rpcs3-interpreter");
        let name = format!("rpcs3-{:?}", Rpcs3Decoder::Llvm).to_lowercase();
        assert_eq!(name, "rpcs3-llvm");
    }

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
