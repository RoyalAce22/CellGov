//! Read-only PowerPC disassembler that delegates decoding to
//! `cellgov_ppu::decode::decode`.
//!
//! Used to investigate guest behavior at specific addresses without
//! booting the title. Output format: `addr  raw  decoded` per
//! instruction. The instruction stream goes to stdout; structural
//! diagnostics ("past segment end", overlap warnings, data heuristic)
//! go to stderr so a downstream tool can pipe stdout cleanly.

use std::io::Write;

use crate::cli::exit::die;

/// Hard cap on `--count`. PPC instructions are 4 bytes, so 1<<16 lines
/// covers a 256 KB code region -- larger than any function-sized
/// investigation this tool exists to support.
const MAX_COUNT: usize = 1 << 16;

/// Number of consecutive `decode` failures after which the user almost
/// certainly pointed the disassembler at data, not code. One stderr
/// note per run.
const CONSECUTIVE_DECODE_NOTE_THRESHOLD: usize = 8;

/// Process exit code when at least one decoded word was an unsupported
/// encoding. Distinct from 1 (fatal CLI error via `die`) so wrappers
/// can tell apart "bad inputs" from "decoded the bytes; some weren't
/// instructions".
const DECODE_ERROR_EXIT_CODE: i32 = 2;

pub(crate) fn run(args: &[String]) {
    let parsed = parse_args(args).unwrap_or_else(|e| die(&e.message()));
    let elf_bytes = std::fs::read(parsed.elf_path)
        .unwrap_or_else(|e| die(&format!("read elf {}: {e}", parsed.elf_path)));
    let segments = parse_pt_loads(&elf_bytes).unwrap_or_else(|e| die(&e.message()));

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stats = match disassemble(&elf_bytes, &segments, parsed.vaddr, parsed.count, &mut out) {
        Ok(s) => s,
        Err(e) => die(&e.message()),
    };

    // Contract: exit code DECODE_ERROR_EXIT_CODE iff at least one
    // word failed to decode. A boundary marker (BSS / past-end) on
    // its own is not an error.
    if stats.decode_errors > 0 {
        std::process::exit(DECODE_ERROR_EXIT_CODE);
    }
}

fn usage() -> &'static str {
    "usage: cellgov_cli disasm <elf-path> --vaddr <hex> [--count N]\n\
     \t--vaddr  hex address (with or without 0x prefix); must be 4-byte aligned\n\
     \t--count  decimal instruction count, 1..=65536, default 16"
}

#[derive(Debug)]
struct DisasmArgs<'a> {
    elf_path: &'a str,
    vaddr: u64,
    count: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ArgError {
    Usage,
    MissingValueFor(&'static str),
    InvalidHex { flag: &'static str, value: String },
    InvalidCount(String),
    CountIsZero,
    CountTooLarge(usize),
    UnalignedVaddr(u64),
    UnknownFlag(String),
}

impl ArgError {
    fn message(&self) -> String {
        match self {
            Self::Usage => usage().to_string(),
            Self::MissingValueFor(flag) => format!("{flag} requires a value\n{}", usage()),
            Self::InvalidHex { flag, value } => {
                format!("invalid {flag}: {value} (expected hex u64; with or without 0x prefix)")
            }
            Self::InvalidCount(value) => {
                format!("invalid --count: {value} (decimal usize)")
            }
            Self::CountIsZero => "--count must be >= 1".to_string(),
            Self::CountTooLarge(n) => format!("--count {n} exceeds maximum {MAX_COUNT}"),
            Self::UnalignedVaddr(v) => format!(
                "--vaddr 0x{v:x} is not 4-byte aligned; PowerPC instructions are aligned words"
            ),
            Self::UnknownFlag(s) => format!("unknown disasm flag: {s}\n{}", usage()),
        }
    }
}

fn parse_args(args: &[String]) -> Result<DisasmArgs<'_>, ArgError> {
    let elf_path = args.get(2).map(String::as_str).ok_or(ArgError::Usage)?;
    let mut vaddr: Option<u64> = None;
    let mut count: usize = 16;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--vaddr" => {
                let v = args
                    .get(i + 1)
                    .ok_or(ArgError::MissingValueFor("--vaddr"))?;
                vaddr = Some(parse_hex_u64(v).ok_or_else(|| ArgError::InvalidHex {
                    flag: "--vaddr",
                    value: v.clone(),
                })?);
                i += 2;
            }
            "--count" => {
                let v = args
                    .get(i + 1)
                    .ok_or(ArgError::MissingValueFor("--count"))?;
                count = v.parse().map_err(|_| ArgError::InvalidCount(v.clone()))?;
                i += 2;
            }
            other => return Err(ArgError::UnknownFlag(other.to_string())),
        }
    }
    let vaddr = vaddr.ok_or(ArgError::Usage)?;
    if !vaddr.is_multiple_of(4) {
        return Err(ArgError::UnalignedVaddr(vaddr));
    }
    if count == 0 {
        return Err(ArgError::CountIsZero);
    }
    if count > MAX_COUNT {
        return Err(ArgError::CountTooLarge(count));
    }
    Ok(DisasmArgs {
        elf_path,
        vaddr,
        count,
    })
}

fn parse_hex_u64(s: &str) -> Option<u64> {
    let trimmed = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(trimmed, 16).ok()
}

#[derive(Debug, PartialEq, Eq)]
enum ElfError {
    TooSmall {
        len: usize,
    },
    BadMagic,
    NotElf64 {
        ei_class: u8,
    },
    NotBigEndian {
        ei_data: u8,
    },
    PhentsizeTooSmall {
        phentsize: u16,
    },
    PhdrCountExtended,
    PhdrTableOverflow {
        phoff: u64,
        phnum: u16,
        phentsize: u16,
    },
    PhdrOutOfFile {
        phoff: u64,
        phend: u64,
        file_len: u64,
    },
    SegmentRangeOverflow {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
    },
    SegmentTruncated {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
        file_len: u64,
    },
}

impl ElfError {
    fn message(&self) -> String {
        match self {
            Self::TooSmall { len } => {
                format!("not an ELF (file is {len} bytes; need >= 64)")
            }
            Self::BadMagic => "not an ELF (magic mismatch)".to_string(),
            Self::NotElf64 { ei_class } => format!(
                "ELF EI_CLASS=0x{ei_class:02x}; this tool only handles ELFCLASS64 (PS3 PPE objects)"
            ),
            Self::NotBigEndian { ei_data } => format!(
                "ELF EI_DATA=0x{ei_data:02x}; this tool only handles ELFDATA2MSB (PS3 PPE objects)"
            ),
            Self::PhentsizeTooSmall { phentsize } => format!(
                "ELF e_phentsize={phentsize} is smaller than Elf64_Phdr (56)"
            ),
            Self::PhdrCountExtended => {
                "ELF e_phnum=0xFFFF (PN_XNUM extension) is not supported by this tool".to_string()
            }
            Self::PhdrTableOverflow {
                phoff,
                phnum,
                phentsize,
            } => format!(
                "ELF program-header arithmetic overflows: phoff=0x{phoff:x} phnum={phnum} phentsize={phentsize}"
            ),
            Self::PhdrOutOfFile {
                phoff,
                phend,
                file_len,
            } => format!(
                "ELF program-header table runs past file: phoff=0x{phoff:x} end=0x{phend:x} file_len=0x{file_len:x}"
            ),
            Self::SegmentRangeOverflow {
                idx,
                p_offset,
                p_filesz,
            } => format!(
                "PT_LOAD #{idx} arithmetic overflows: p_offset=0x{p_offset:x} p_filesz=0x{p_filesz:x}"
            ),
            Self::SegmentTruncated {
                idx,
                p_offset,
                p_filesz,
                file_len,
            } => format!(
                "PT_LOAD #{idx} truncated: p_offset=0x{p_offset:x}+p_filesz=0x{p_filesz:x} runs past file_len=0x{file_len:x}"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PtLoad {
    vaddr: u64,
    offset: u64,
    filesz: u64,
    memsz: u64,
}

/// Parse all PT_LOAD program headers out of an ELF64-BE blob.
///
/// Validates EI_CLASS and EI_DATA, the program-header table extent,
/// `e_phentsize`, `e_phnum != PN_XNUM`, and that each PT_LOAD's
/// `[p_offset, p_offset + p_filesz)` lies entirely inside the file.
/// `disassemble` relies on those checks to skip per-byte bounds
/// validation in the hot loop.
fn parse_pt_loads(data: &[u8]) -> Result<Vec<PtLoad>, ElfError> {
    if data.len() < 64 {
        return Err(ElfError::TooSmall { len: data.len() });
    }
    if &data[0..4] != b"\x7fELF" {
        return Err(ElfError::BadMagic);
    }
    if data[4] != 2 {
        return Err(ElfError::NotElf64 { ei_class: data[4] });
    }
    if data[5] != 2 {
        return Err(ElfError::NotBigEndian { ei_data: data[5] });
    }

    let phoff = read_be_u64(data, 32);
    let phentsize = u16::from_be_bytes([data[54], data[55]]);
    let phnum = u16::from_be_bytes([data[56], data[57]]);

    if phnum == 0xFFFF {
        return Err(ElfError::PhdrCountExtended);
    }
    if (phentsize as usize) < 56 {
        return Err(ElfError::PhentsizeTooSmall { phentsize });
    }

    let table_size =
        (phnum as u64)
            .checked_mul(phentsize as u64)
            .ok_or(ElfError::PhdrTableOverflow {
                phoff,
                phnum,
                phentsize,
            })?;
    let phend = phoff
        .checked_add(table_size)
        .ok_or(ElfError::PhdrTableOverflow {
            phoff,
            phnum,
            phentsize,
        })?;
    if phend > data.len() as u64 {
        return Err(ElfError::PhdrOutOfFile {
            phoff,
            phend,
            file_len: data.len() as u64,
        });
    }

    let mut out = Vec::new();
    for i in 0..phnum as usize {
        let base = phoff as usize + i * phentsize as usize;
        let p_type =
            u32::from_be_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        if p_type != 1 {
            continue;
        }
        let p_offset = read_be_u64(data, base + 8);
        let p_vaddr = read_be_u64(data, base + 16);
        let p_filesz = read_be_u64(data, base + 32);
        let p_memsz = read_be_u64(data, base + 40);

        let seg_end_in_file =
            p_offset
                .checked_add(p_filesz)
                .ok_or(ElfError::SegmentRangeOverflow {
                    idx: i,
                    p_offset,
                    p_filesz,
                })?;
        if seg_end_in_file > data.len() as u64 {
            return Err(ElfError::SegmentTruncated {
                idx: i,
                p_offset,
                p_filesz,
                file_len: data.len() as u64,
            });
        }
        out.push(PtLoad {
            vaddr: p_vaddr,
            offset: p_offset,
            filesz: p_filesz,
            memsz: p_memsz,
        });
    }
    Ok(out)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DisasmStats {
    /// Total lines written to the instruction stream, including the
    /// terminal `<past segment end>` / `<BSS / zero-fill>` markers.
    /// Real instruction lines are `lines_written - markers_written`.
    lines_written: usize,
    /// Boundary marker lines (BSS or past-segment-end) written. At
    /// most one per run.
    markers_written: usize,
    /// Number of `decode` failures encountered.
    decode_errors: usize,
    /// True if the consecutive-failure heuristic note fired.
    data_warning_emitted: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum DisasmError {
    VaddrNotInPtLoad { vaddr: u64, segments: Vec<PtLoad> },
    VaddrInBssOnly { vaddr: u64, seg: PtLoad },
}

impl DisasmError {
    fn message(&self) -> String {
        match self {
            Self::VaddrNotInPtLoad { vaddr, segments } => {
                let segs = segments
                    .iter()
                    .map(|s| {
                        format!(
                            "\n  vaddr=0x{:x}+filesz=0x{:x} memsz=0x{:x} file=0x{:x}",
                            s.vaddr, s.filesz, s.memsz, s.offset
                        )
                    })
                    .collect::<String>();
                format!("vaddr 0x{vaddr:x} not in any PT_LOAD; segments:{segs}")
            }
            Self::VaddrInBssOnly { vaddr, seg } => format!(
                "vaddr 0x{vaddr:x} is in PT_LOAD vaddr=0x{:x}+filesz=0x{:x} (memsz=0x{:x}) but past the file-backed range; nothing to disassemble (BSS / zero-fill)",
                seg.vaddr, seg.filesz, seg.memsz
            ),
        }
    }
}

/// Pick the PT_LOAD that file-backs `vaddr`. With overlapping
/// segments (legal but unusual; firmware modules occasionally emit
/// them), pick the smallest containing segment; ties break on lowest
/// `p_offset`, then lowest `p_vaddr`. Two PT_LOADs with identical
/// `(filesz, offset, vaddr)` are indistinguishable by file content,
/// so picking either is correct -- the triple-key sort makes the
/// choice fully a function of the segment data, not phdr order.
/// Emits a stderr note when more than one segment matches.
fn select_segment(segments: &[PtLoad], vaddr: u64) -> Result<PtLoad, DisasmError> {
    let mut candidates: Vec<PtLoad> = segments
        .iter()
        .copied()
        .filter(|s| vaddr >= s.vaddr && vaddr < s.vaddr.saturating_add(s.filesz))
        .collect();
    if candidates.is_empty() {
        let bss_match = segments
            .iter()
            .copied()
            .find(|s| vaddr >= s.vaddr && vaddr < s.vaddr.saturating_add(s.memsz));
        if let Some(seg) = bss_match {
            return Err(DisasmError::VaddrInBssOnly { vaddr, seg });
        }
        return Err(DisasmError::VaddrNotInPtLoad {
            vaddr,
            segments: segments.to_vec(),
        });
    }
    if candidates.len() > 1 {
        eprintln!(
            "note: vaddr 0x{vaddr:x} is in {} overlapping PT_LOADs; choosing the smallest containing segment",
            candidates.len()
        );
    }
    candidates.sort_by_key(|s| (s.filesz, s.offset, s.vaddr));
    Ok(candidates[0])
}

/// Read `count` aligned 32-bit words starting at `vaddr`, decoding
/// each and writing one line per word into `out`.
///
/// Caller is responsible for `vaddr % 4 == 0` and `count > 0`
/// (`parse_args` enforces both). `parse_pt_loads` guarantees that any
/// `seg.offset + off_in_seg + 4 <= elf_bytes.len()` whenever the
/// per-iteration filesz check passes, so the hot loop indexes
/// `elf_bytes` without further bounds checks.
fn disassemble<W: Write>(
    elf_bytes: &[u8],
    segments: &[PtLoad],
    vaddr: u64,
    count: usize,
    out: &mut W,
) -> Result<DisasmStats, DisasmError> {
    debug_assert!(vaddr.is_multiple_of(4), "parse_args must enforce alignment");
    debug_assert!(count > 0, "parse_args must reject count == 0");
    debug_assert!(count <= MAX_COUNT, "parse_args must enforce the count cap");

    let seg = select_segment(segments, vaddr)?;

    let mut stats = DisasmStats::default();
    let mut consecutive = 0usize;

    for n in 0..count {
        let Some(addr) = (n as u64)
            .checked_mul(4)
            .and_then(|delta| vaddr.checked_add(delta))
        else {
            let _ = writeln!(out, "<address overflow at iteration {n}>");
            break;
        };
        let off_in_seg = addr - seg.vaddr;
        let needed_end = off_in_seg.checked_add(4);
        match needed_end {
            Some(end) if end <= seg.filesz => {}
            Some(end) if end <= seg.memsz => {
                let _ = writeln!(
                    out,
                    "0x{addr:016x}  --------  <in PT_LOAD but past filesz (BSS / zero-fill)>"
                );
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
            _ => {
                let _ = writeln!(out, "0x{addr:016x}  --------  <past segment end>");
                stats.lines_written += 1;
                stats.markers_written += 1;
                break;
            }
        }
        let file_off = (seg.offset + off_in_seg) as usize;
        let raw = u32::from_be_bytes([
            elf_bytes[file_off],
            elf_bytes[file_off + 1],
            elf_bytes[file_off + 2],
            elf_bytes[file_off + 3],
        ]);
        match cellgov_ppu::decode::decode(raw) {
            Ok(insn) => {
                consecutive = 0;
                let _ = writeln!(out, "0x{addr:016x}  {raw:08x}  {insn:?}");
                stats.lines_written += 1;
            }
            Err(_) => {
                consecutive += 1;
                stats.decode_errors += 1;
                let _ = writeln!(out, "0x{addr:016x}  {raw:08x}  <unsupported encoding>");
                stats.lines_written += 1;
                if !stats.data_warning_emitted && consecutive >= CONSECUTIVE_DECODE_NOTE_THRESHOLD {
                    eprintln!(
                        "note: {CONSECUTIVE_DECODE_NOTE_THRESHOLD}+ consecutive decode failures; this address may be data, not code"
                    );
                    stats.data_warning_emitted = true;
                }
            }
        }
    }
    Ok(stats)
}

fn read_be_u64(data: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic Elf64_Phdr description for `build_elf64_be`.
    struct SegSpec {
        p_type: u32,
        p_offset: u64,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
        bytes: Vec<u8>,
    }

    impl SegSpec {
        fn pt_load(p_offset: u64, p_vaddr: u64, bytes: Vec<u8>) -> Self {
            let len = bytes.len() as u64;
            Self {
                p_type: 1,
                p_offset,
                p_vaddr,
                p_filesz: len,
                p_memsz: len,
                bytes,
            }
        }
    }

    fn put_be_u16(buf: &mut [u8], off: usize, v: u16) {
        buf[off..off + 2].copy_from_slice(&v.to_be_bytes());
    }
    fn put_be_u32(buf: &mut [u8], off: usize, v: u32) {
        buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn put_be_u64(buf: &mut [u8], off: usize, v: u64) {
        buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
    }

    /// Build an ELF64-MSB blob containing the given segments. Phdrs
    /// land immediately after the 64-byte ELF header; segment bytes
    /// land at each spec's `p_offset`.
    fn build_elf64_be(segs: &[SegSpec]) -> Vec<u8> {
        const PHENTSIZE: u16 = 56;
        let phoff: u64 = 64;
        let phnum: u16 = segs.len() as u16;
        let phdr_table_end = phoff + (PHENTSIZE as u64) * (phnum as u64);
        let mut file_end: u64 = phdr_table_end;
        for seg in segs {
            let segend = seg.p_offset + seg.bytes.len() as u64;
            if segend > file_end {
                file_end = segend;
            }
        }
        let mut data = vec![0u8; file_end as usize];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 2; // EI_CLASS = ELFCLASS64
        data[5] = 2; // EI_DATA  = ELFDATA2MSB
        data[6] = 1; // EI_VERSION
        put_be_u16(&mut data, 16, 2); // e_type = ET_EXEC
        put_be_u16(&mut data, 18, 21); // e_machine = EM_PPC64
        put_be_u32(&mut data, 20, 1); // e_version
        put_be_u64(&mut data, 32, phoff);
        put_be_u16(&mut data, 52, 64); // e_ehsize
        put_be_u16(&mut data, 54, PHENTSIZE);
        put_be_u16(&mut data, 56, phnum);

        for (i, seg) in segs.iter().enumerate() {
            let base = phoff as usize + i * PHENTSIZE as usize;
            put_be_u32(&mut data, base, seg.p_type);
            put_be_u32(&mut data, base + 4, 5); // p_flags = PF_R | PF_X
            put_be_u64(&mut data, base + 8, seg.p_offset);
            put_be_u64(&mut data, base + 16, seg.p_vaddr);
            put_be_u64(&mut data, base + 24, seg.p_vaddr); // p_paddr
            put_be_u64(&mut data, base + 32, seg.p_filesz);
            put_be_u64(&mut data, base + 40, seg.p_memsz);
            put_be_u64(&mut data, base + 48, 0); // p_align
            let off = seg.p_offset as usize;
            data[off..off + seg.bytes.len()].copy_from_slice(&seg.bytes);
        }
        data
    }

    /// `nop` (ori 0,0,0) = 0x60000000.
    const NOP: [u8; 4] = [0x60, 0x00, 0x00, 0x00];
    /// `blr` = 0x4E800020.
    const BLR: [u8; 4] = [0x4E, 0x80, 0x00, 0x20];

    fn args_vec(extra: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = ["cellgov_cli", "disasm", "/tmp/elf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for s in extra {
            v.push(s.to_string());
        }
        v
    }

    // -- parse_hex_u64 --

    #[test]
    fn parse_hex_accepts_with_and_without_prefix() {
        assert_eq!(parse_hex_u64("0x10"), Some(0x10));
        assert_eq!(parse_hex_u64("0X10"), Some(0x10));
        assert_eq!(parse_hex_u64("10"), Some(0x10));
        assert_eq!(parse_hex_u64("deadbeef"), Some(0xdead_beef));
    }

    #[test]
    fn parse_hex_rejects_garbage() {
        assert_eq!(parse_hex_u64(""), None);
        assert_eq!(parse_hex_u64("0x"), None);
        assert_eq!(parse_hex_u64("0xZZ"), None);
        assert_eq!(parse_hex_u64("ffffffffffffffff0"), None); // overflow
    }

    // -- parse_args --

    #[test]
    fn parse_args_requires_vaddr() {
        let err = parse_args(&args_vec(&[])).unwrap_err();
        assert_eq!(err, ArgError::Usage);
    }

    #[test]
    fn parse_args_rejects_unaligned_vaddr() {
        let err = parse_args(&args_vec(&["--vaddr", "0x10002"])).unwrap_err();
        assert_eq!(err, ArgError::UnalignedVaddr(0x10002));
    }

    #[test]
    fn parse_args_rejects_count_zero() {
        let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count", "0"])).unwrap_err();
        assert_eq!(err, ArgError::CountIsZero);
    }

    #[test]
    fn parse_args_rejects_count_over_max() {
        let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count", "1000000"])).unwrap_err();
        assert_eq!(err, ArgError::CountTooLarge(1_000_000));
    }

    #[test]
    fn parse_args_reports_missing_value_for_specific_flag() {
        let err = parse_args(&args_vec(&["--vaddr"])).unwrap_err();
        assert_eq!(err, ArgError::MissingValueFor("--vaddr"));
        let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--count"])).unwrap_err();
        assert_eq!(err, ArgError::MissingValueFor("--count"));
    }

    #[test]
    fn parse_args_unknown_flag_is_specific() {
        let err = parse_args(&args_vec(&["--vaddr", "0x10000", "--lol"])).unwrap_err();
        assert_eq!(err, ArgError::UnknownFlag("--lol".to_string()));
    }

    #[test]
    fn parse_args_invalid_hex_includes_value() {
        let err = parse_args(&args_vec(&["--vaddr", "nothex!"])).unwrap_err();
        assert_eq!(
            err,
            ArgError::InvalidHex {
                flag: "--vaddr",
                value: "nothex!".to_string()
            }
        );
    }

    #[test]
    fn parse_args_happy_path() {
        let argv = args_vec(&["--vaddr", "0x10000", "--count", "32"]);
        let p = parse_args(&argv).unwrap();
        assert_eq!(p.vaddr, 0x10000);
        assert_eq!(p.count, 32);
        assert_eq!(p.elf_path, "/tmp/elf");
    }

    // -- parse_pt_loads --

    #[test]
    fn pt_loads_rejects_too_small() {
        assert_eq!(
            parse_pt_loads(&[0u8; 32]),
            Err(ElfError::TooSmall { len: 32 })
        );
    }

    #[test]
    fn pt_loads_rejects_bad_magic() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"NOPE");
        assert_eq!(parse_pt_loads(&data), Err(ElfError::BadMagic));
    }

    #[test]
    fn pt_loads_rejects_elfclass32() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        data[4] = 1; // ELFCLASS32
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::NotElf64 { ei_class: 1 })
        );
    }

    #[test]
    fn pt_loads_rejects_little_endian() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        data[5] = 1; // ELFDATA2LSB
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::NotBigEndian { ei_data: 1 })
        );
    }

    #[test]
    fn pt_loads_rejects_pn_xnum() {
        let mut data = build_elf64_be(&[]);
        put_be_u16(&mut data, 56, 0xFFFF);
        assert_eq!(parse_pt_loads(&data), Err(ElfError::PhdrCountExtended));
    }

    #[test]
    fn pt_loads_rejects_phentsize_too_small() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        put_be_u16(&mut data, 54, 32);
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::PhentsizeTooSmall { phentsize: 32 })
        );
    }

    #[test]
    fn pt_loads_rejects_phdr_running_past_file() {
        let mut data = build_elf64_be(&[]);
        // Claim 1000 phdrs starting at offset 64; nowhere near enough file.
        put_be_u16(&mut data, 56, 1000);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::PhdrOutOfFile { .. }) => {}
            other => panic!("expected PhdrOutOfFile, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_segment_truncated_in_file() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        // Inflate p_filesz so seg_end_in_file > data.len()
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 32, 0x10_0000);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::SegmentTruncated { idx: 0, .. }) => {}
            other => panic!("expected SegmentTruncated, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_skips_non_pt_load_entries() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_type = 0x6474_E551; // PT_GNU_STACK
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        assert!(segs.is_empty());
    }

    // -- disassemble --

    fn nop_elf() -> (Vec<u8>, Vec<PtLoad>) {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&NOP);
        }
        bytes.extend_from_slice(&BLR);
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        (data, segs)
    }

    #[test]
    fn disassemble_decodes_aligned_words() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 5, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 0);
        assert_eq!(stats.lines_written, 5);
        assert_eq!(stats.markers_written, 0);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("60000000"), "stream missing nop word: {text}");
        assert!(text.contains("4e800020"), "stream missing blr word: {text}");
    }

    #[test]
    fn disassemble_address_column_uses_16_hex_digits() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        disassemble(&data, &segs, 0x10000, 1, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("0x0000000000010000"),
            "expected zero-padded 16-hex-digit address, got: {text}"
        );
    }

    #[test]
    fn disassemble_rejects_vaddr_outside_pt_load() {
        let (data, segs) = nop_elf();
        let mut out = Vec::new();
        let err = disassemble(&data, &segs, 0x90000, 1, &mut out).unwrap_err();
        match err {
            DisasmError::VaddrNotInPtLoad { vaddr: 0x90000, .. } => {}
            other => panic!("expected VaddrNotInPtLoad, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_distinguishes_bss_from_outside_segment() {
        // PT_LOAD with filesz=4 but memsz=12: addresses in [vaddr+4, vaddr+12)
        // are in the segment but past the file-backed bytes.
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_memsz = 12;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let err = disassemble(&data, &segs, 0x10004, 1, &mut out).unwrap_err();
        match err {
            DisasmError::VaddrInBssOnly { vaddr: 0x10004, .. } => {}
            other => panic!("expected VaddrInBssOnly, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_marks_bss_when_iterating_into_zero_fill() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_memsz = 12;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 4, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        // First word decodes; second iteration crosses into BSS.
        assert!(text.contains("BSS / zero-fill"), "stream:\n{text}");
        assert_eq!(stats.lines_written, 2);
        assert_eq!(stats.markers_written, 1);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        assert_eq!(stats.decode_errors, 0);
    }

    #[test]
    fn disassemble_marks_past_segment_end_when_outside_memsz_too() {
        let (data, segs) = nop_elf();
        // Segment is 5 instructions (20 bytes); ask for 8.
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 8, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("past segment end"), "stream:\n{text}");
        assert_eq!(stats.lines_written, 6);
        assert_eq!(stats.markers_written, 1);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
    }

    /// Primary opcode 1 has no top-level arm in `cellgov_ppu::decode`,
    /// so any word of the form `0x04xxxxxx` returns
    /// `PpuDecodeError::Unsupported`. 0xFFFFFFFF would NOT work --
    /// primary 63 routes to `Fp63`.
    const UNSUPPORTED_WORD: [u8; 4] = [0x04, 0x00, 0x00, 0x00];

    #[test]
    fn disassemble_consecutive_decode_failures_emit_warning() {
        let mut bytes = Vec::new();
        for _ in 0..16 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 16, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 16);
        assert!(stats.data_warning_emitted);
    }

    #[test]
    fn disassemble_resets_consecutive_counter_on_success() {
        // 4 unsupported, then 4 nops, then 4 unsupported. Max consecutive
        // run is 4, below the threshold; the warning must NOT fire.
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&NOP);
        }
        for _ in 0..4 {
            bytes.extend_from_slice(&UNSUPPORTED_WORD);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10000, 12, &mut out).unwrap();
        assert_eq!(stats.decode_errors, 8);
        assert!(!stats.data_warning_emitted);
    }

    #[test]
    fn select_segment_picks_smallest_containing_when_overlapping() {
        let big = PtLoad {
            vaddr: 0x10000,
            offset: 0x200,
            filesz: 0x1000,
            memsz: 0x1000,
        };
        let small = PtLoad {
            vaddr: 0x10000,
            offset: 0x4000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let chosen = select_segment(&[big, small], 0x10000).unwrap();
        assert_eq!(chosen, small);
    }

    #[test]
    fn select_segment_breaks_filesz_ties_by_offset_then_vaddr() {
        // Three equally-sized overlapping segments. Lowest p_offset wins.
        let a = PtLoad {
            vaddr: 0x10000,
            offset: 0x4000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let b = PtLoad {
            vaddr: 0x10000,
            offset: 0x2000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let c = PtLoad {
            vaddr: 0x10000,
            offset: 0x6000,
            filesz: 0x100,
            memsz: 0x100,
        };
        let chosen = select_segment(&[a, b, c], 0x10000).unwrap();
        assert_eq!(chosen, b);
    }

    // -- precondition: the sentinel really is unsupported --

    #[test]
    fn unsupported_word_constant_is_actually_unsupported() {
        // If `cellgov_ppu::decode` ever adds an arm covering primary
        // opcode 1, the heuristic tests below silently flip from
        // "exercises the failure path" to "exercises the success
        // path." Fail loudly here so the maintainer picks a new
        // sentinel rather than letting the heuristic tests go vacuous.
        let raw = u32::from_be_bytes(UNSUPPORTED_WORD);
        assert!(
            cellgov_ppu::decode::decode(raw).is_err(),
            "UNSUPPORTED_WORD ({raw:#010x}) decoded successfully; pick a different sentinel"
        );
    }

    // -- missing error-variant coverage --

    #[test]
    fn pt_loads_rejects_phdr_table_arithmetic_overflow() {
        // phoff=u64::MAX-10, phnum=1, phentsize=56 -> phoff+table_size overflows u64.
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        put_be_u64(&mut data, 32, u64::MAX - 10);
        put_be_u16(&mut data, 56, 1);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::PhdrTableOverflow { .. }) => {}
            other => panic!("expected PhdrTableOverflow, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_segment_range_overflow() {
        // Place a single PT_LOAD, then poke its p_offset to u64::MAX
        // and p_filesz to 1 so checked_add overflows.
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 8, u64::MAX);
        put_be_u64(&mut data, phdr_base + 32, 1);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::SegmentRangeOverflow { idx: 0, .. }) => {}
            other => panic!("expected SegmentRangeOverflow, got {other:?}"),
        }
    }

    // -- boundary cases --

    #[test]
    fn parse_args_accepts_count_at_max() {
        let argv = args_vec(&["--vaddr", "0x10000", "--count", "65536"]);
        let p = parse_args(&argv).unwrap();
        assert_eq!(p.count, MAX_COUNT);
    }

    #[test]
    fn disassemble_decodes_last_word_in_segment() {
        // Segment of exactly two instructions; ask for the last word
        // alone. Must decode cleanly with no marker.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&NOP);
        bytes.extend_from_slice(&BLR);
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let mut out = Vec::new();
        let stats = disassemble(&data, &segs, 0x10004, 1, &mut out).unwrap();
        assert_eq!(stats.lines_written, 1);
        assert_eq!(stats.markers_written, 0);
        assert!(
            stats.markers_written <= 1,
            "markers_written must never exceed 1"
        );
        assert_eq!(stats.decode_errors, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("4e800020"), "stream:\n{text}");
    }

    #[test]
    fn select_segment_rejects_vaddr_at_exclusive_upper_bound() {
        // Segment [0x10000, 0x10010); vaddr=0x10010 is one past the
        // end and must NOT be considered inside the segment.
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&NOP);
        }
        let data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, bytes)]);
        let segs = parse_pt_loads(&data).unwrap();
        let err = select_segment(&segs, 0x10010).unwrap_err();
        match err {
            DisasmError::VaddrNotInPtLoad { vaddr: 0x10010, .. } => {}
            other => panic!("expected VaddrNotInPtLoad, got {other:?}"),
        }
    }

    #[test]
    fn select_segment_at_exclusive_filesz_upper_bound_with_bigger_memsz_is_bss() {
        // Segment vaddr=[0x10000, 0x10004) by filesz, [0x10000, 0x10010) by memsz.
        // vaddr=0x10004 is at the exclusive filesz boundary but inside memsz: BSS.
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_memsz = 16;
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        let err = select_segment(&segs, 0x10004).unwrap_err();
        match err {
            DisasmError::VaddrInBssOnly { vaddr: 0x10004, .. } => {}
            other => panic!("expected VaddrInBssOnly, got {other:?}"),
        }
    }
}
