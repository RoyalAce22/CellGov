//! Checkpoint observation capture for `run-game --save-observation`.
//!
//! `CheckpointManifest` shares the schema `bridges/rpcs3_to_observation/`
//! consumes so both runners read the same TOML when comparing runs.

use serde::Deserialize;

/// Region list in a checkpoint observation manifest.
#[derive(Debug, Deserialize)]
pub(super) struct CheckpointManifest {
    pub(super) regions: Vec<CheckpointRegion>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckpointRegion {
    pub(super) name: String,
    #[serde(deserialize_with = "de_hex_u64")]
    pub(super) addr: u64,
    #[serde(deserialize_with = "de_hex_u64")]
    pub(super) size: u64,
}

/// Highest end address of any PT_LOAD segment whose vaddr falls in
/// `[0x00010000, 0x10000000)`. Segments above that range share no
/// address space with `sys_memory_allocate` and do not advance the
/// allocator base.
///
/// Returns 0 with a distinct stderr line for each failure mode
/// (short input, bad magic, truncated phdr table, no user segments).
pub(super) fn elf_user_region_end(data: &[u8]) -> usize {
    const PT_LOAD: u32 = 1;
    fn u16_be(d: &[u8], o: usize) -> u16 {
        u16::from_be_bytes([d[o], d[o + 1]])
    }
    fn u32_be(d: &[u8], o: usize) -> u32 {
        u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
    }
    fn u64_be(d: &[u8], o: usize) -> u64 {
        u64::from_be_bytes([
            d[o],
            d[o + 1],
            d[o + 2],
            d[o + 3],
            d[o + 4],
            d[o + 5],
            d[o + 6],
            d[o + 7],
        ])
    }
    if data.len() < 64 {
        eprintln!(
            "elf_user_region_end: input too short for ELF64 header ({} bytes); returning 0",
            data.len()
        );
        return 0;
    }
    if data[0..4] != [0x7f, 0x45, 0x4c, 0x46] {
        eprintln!("elf_user_region_end: ELF magic mismatch; returning 0");
        return 0;
    }
    let phoff = u64_be(data, 32) as usize;
    let phentsize = u16_be(data, 54) as usize;
    let phnum = u16_be(data, 56) as usize;
    // Up-front bound check: a corrupted phnum that overflows
    // data.len() must not mid-scan `break` and silently truncate.
    let ph_table_end = phoff.saturating_add(phentsize.saturating_mul(phnum));
    if ph_table_end > data.len() {
        eprintln!(
            "elf_user_region_end: program header table (phoff=0x{phoff:x} phentsize={phentsize} phnum={phnum}) extends past end-of-file ({} bytes); returning 0",
            data.len()
        );
        return 0;
    }
    let mut max_end: usize = 0;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if u32_be(data, base) != PT_LOAD {
            continue;
        }
        let p_vaddr = u64_be(data, base + 16) as usize;
        let p_memsz = u64_be(data, base + 40) as usize;
        if p_memsz == 0 {
            continue;
        }
        if (0x0001_0000..0x1000_0000).contains(&p_vaddr) {
            let end = p_vaddr + p_memsz;
            if end > max_end {
                max_end = end;
            }
        }
    }
    max_end
}

fn de_hex_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    let trimmed = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(trimmed, 16).map_err(serde::de::Error::custom)
}

/// Build a boot-checkpoint observation and write it as JSON.
///
/// Regions default to one per PT_LOAD segment, named
/// `seg{index}_{ro|rw}`. With `manifest_path`, regions come from
/// the TOML manifest instead -- cross-runner comparison relies on
/// both runners reading the same file for matching region names.
///
/// # Errors
///
/// Returns `Err(message)` on any I/O, parse, or serialization
/// failure so the caller can translate it to a non-zero exit.
pub(super) fn save_boot_observation(
    path: &str,
    elf_data: &[u8],
    final_memory: &[u8],
    outcome: cellgov_compare::BootOutcome,
    steps: usize,
    manifest_path: Option<&str>,
) -> Result<(), String> {
    let regions: Vec<cellgov_compare::RegionDescriptor> = match manifest_path {
        Some(mp) => {
            let manifest: CheckpointManifest = std::fs::read_to_string(mp)
                .map_err(|e| format!("read {mp}: {e}"))
                .and_then(|t| {
                    toml::from_str::<CheckpointManifest>(&t).map_err(|e| format!("parse {mp}: {e}"))
                })?;
            manifest
                .regions
                .into_iter()
                .map(|r| cellgov_compare::RegionDescriptor {
                    name: r.name,
                    addr: r.addr,
                    size: r.size,
                })
                .collect()
        }
        None => {
            let segments = cellgov_ppu::loader::pt_load_segments(elf_data)
                .map_err(|e| format!("failed to enumerate PT_LOAD: {e:?}"))?;
            segments
                .iter()
                .map(|s| {
                    let kind = if s.writable { "rw" } else { "ro" };
                    cellgov_compare::RegionDescriptor {
                        name: format!("seg{}_{kind}", s.index),
                        addr: s.vaddr,
                        size: s.memsz,
                    }
                })
                .collect()
        }
    };
    let observation = cellgov_compare::observe_from_boot(final_memory, outcome, steps, &regions);
    let json =
        serde_json::to_string_pretty(&observation).map_err(|e| format!("serialize failed: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write to {path} failed: {e}"))?;
    println!(
        "observation: wrote {} regions covering {} bytes to {path}",
        observation.memory_regions.len(),
        observation
            .memory_regions
            .iter()
            .map(|r| r.data.len())
            .sum::<usize>(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Big-endian ELF64 header with N PT_LOAD phdrs at the given
    /// (vaddr, memsz) tuples. Payloads are not materialized.
    fn synthetic_elf(loads: &[(u64, u64)]) -> Vec<u8> {
        let phoff: u64 = 64;
        let phentsize: u16 = 56;
        let phnum: u16 = loads.len() as u16;
        let header_end = phoff as usize + phentsize as usize * phnum as usize;
        let mut buf = vec![0u8; header_end];
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        buf[4] = 2; // ELFCLASS64
        buf[5] = 2; // ELFDATA2MSB (big-endian)
        buf[32..40].copy_from_slice(&phoff.to_be_bytes());
        buf[54..56].copy_from_slice(&phentsize.to_be_bytes());
        buf[56..58].copy_from_slice(&phnum.to_be_bytes());
        for (i, &(vaddr, memsz)) in loads.iter().enumerate() {
            let base = phoff as usize + i * phentsize as usize;
            buf[base..base + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
            buf[base + 16..base + 24].copy_from_slice(&vaddr.to_be_bytes());
            buf[base + 40..base + 48].copy_from_slice(&memsz.to_be_bytes());
        }
        buf
    }

    #[test]
    fn elf_user_region_end_picks_max_in_user_range() {
        let elf = synthetic_elf(&[(0x0001_0000, 0x80_0000), (0x0082_0000, 0x7_5CD4)]);
        assert_eq!(elf_user_region_end(&elf), 0x0082_0000 + 0x7_5CD4);
    }

    #[test]
    fn elf_user_region_end_ignores_segments_above_user_range() {
        let elf = synthetic_elf(&[
            (0x0001_0000, 0x10_0000),
            (0x1000_0000, 0x4_0000),
            (0x1006_0000, 0x100),
        ]);
        assert_eq!(elf_user_region_end(&elf), 0x0001_0000 + 0x10_0000);
    }

    #[test]
    fn elf_user_region_end_skips_zero_memsz() {
        let elf = synthetic_elf(&[(0x0001_0000, 0), (0x0002_0000, 0x100)]);
        assert_eq!(elf_user_region_end(&elf), 0x0002_0000 + 0x100);
    }

    #[test]
    fn elf_user_region_end_returns_zero_for_no_user_segments() {
        let elf = synthetic_elf(&[(0x1000_0000, 0x4_0000)]);
        assert_eq!(elf_user_region_end(&elf), 0);
    }

    #[test]
    fn elf_user_region_end_rejects_non_elf_input() {
        assert_eq!(elf_user_region_end(&[0u8; 64]), 0);
        assert_eq!(elf_user_region_end(&[0u8; 4]), 0);
    }

    fn parse(text: &str) -> CheckpointManifest {
        toml::from_str(text).expect("parses")
    }

    #[test]
    fn checkpoint_manifest_parses_hex_addresses() {
        let m = parse(
            r#"
            [[regions]]
            name = "code"
            addr = "0x10000"
            size = "0x800000"

            [[regions]]
            name = "rodata"
            addr = "0x10000000"
            size = "0x40000"
            "#,
        );
        assert_eq!(m.regions.len(), 2);
        let CheckpointRegion {
            ref name,
            addr,
            size,
        } = m.regions[0];
        assert_eq!(name, "code");
        assert_eq!(addr, 0x10000);
        assert_eq!(size, 0x800000);
        assert_eq!(m.regions[1].addr, 0x1000_0000);
        assert_eq!(m.regions[1].size, 0x40000);
    }

    #[test]
    fn checkpoint_manifest_accepts_unprefixed_hex() {
        let m = parse(
            r#"
            [[regions]]
            name = "r"
            addr = "1000"
            size = "10"
            "#,
        );
        assert_eq!(m.regions[0].addr, 0x1000);
        assert_eq!(m.regions[0].size, 0x10);
    }

    #[test]
    fn checkpoint_manifest_rejects_non_hex_value() {
        let bad = toml::from_str::<CheckpointManifest>(
            r#"
            [[regions]]
            name = "r"
            addr = "not-hex"
            size = "10"
            "#,
        );
        assert!(bad.is_err(), "non-hex addr must fail");
    }

    #[test]
    fn checkpoint_manifest_loads_committed_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
            .join("NPUA80001_checkpoint.toml");
        let text = std::fs::read_to_string(&path).expect("read");
        let m: CheckpointManifest = toml::from_str(&text).expect("parses");
        assert!(!m.regions.is_empty());
        assert!(m.regions.iter().any(|r| r.name == "code"));
    }
}
