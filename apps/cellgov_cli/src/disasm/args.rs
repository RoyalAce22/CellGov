//! CLI flag parsing for the `disasm` subcommand.
//!
//! Owns the positional + `--vaddr` + `--count` parse, the typed
//! [`ArgError`] surface, and the `MAX_COUNT` ceiling. Enforces
//! 4-byte vaddr alignment and the 1..=65536 count range so
//! downstream callers can treat their inputs as already validated.

/// Hard cap on `--count`. PPC instructions are 4 bytes, so 1<<16 lines
/// covers a 256 KB code region -- larger than any function-sized
/// investigation this tool exists to support.
pub(super) const MAX_COUNT: usize = 1 << 16;

pub(super) fn usage() -> &'static str {
    "usage: cellgov_cli disasm <elf-path> --vaddr <hex> [--count N] [--symbolize] [--vfs-root PATH]\n\
     \t--vaddr      hex address (with or without 0x prefix); must be 4-byte aligned\n\
     \t--count      decimal instruction count, 1..=65536, default 16\n\
     \t--symbolize  build the OPD function map and annotate branch targets\n\
     \t--vfs-root   PS3 vfs root for NPDRM RAP lookup (default: tools/rpcs3/dev_hdd0)"
}

#[derive(Debug)]
pub(super) struct DisasmArgs<'a> {
    pub(super) elf_path: &'a str,
    pub(super) vaddr: u64,
    pub(super) count: usize,
    pub(super) symbolize: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum ArgError {
    #[error("{}", usage())]
    Usage,
    #[error("{_0} requires a value\n{}", usage())]
    MissingValueFor(&'static str),
    #[error("invalid {flag}: {value} (expected hex u64; with or without 0x prefix)")]
    InvalidHex { flag: &'static str, value: String },
    #[error("invalid --count: {_0} (decimal usize)")]
    InvalidCount(String),
    #[error("--count must be >= 1")]
    CountIsZero,
    #[error("--count {_0} exceeds maximum {}", MAX_COUNT)]
    CountTooLarge(usize),
    #[error("--vaddr 0x{_0:016x} is not 4-byte aligned; PowerPC instructions are aligned words")]
    UnalignedVaddr(u64),
    #[error("unknown disasm flag: {_0}\n{}", usage())]
    UnknownFlag(String),
}

impl ArgError {
    pub(super) fn message(&self) -> String {
        self.to_string()
    }
}

pub(super) fn parse_args(args: &[String]) -> Result<DisasmArgs<'_>, ArgError> {
    let elf_path = args.get(2).map(String::as_str).ok_or(ArgError::Usage)?;
    let mut vaddr: Option<u64> = None;
    let mut count: usize = 16;
    let mut symbolize = false;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--symbolize" => {
                symbolize = true;
                i += 1;
            }
            // Consumed so it is not rejected as unknown; the value is
            // re-read by `resolve_ps3_vfs_root` for RAP resolution.
            "--vfs-root" => {
                if args.get(i + 1).is_none() {
                    return Err(ArgError::MissingValueFor("--vfs-root"));
                }
                i += 2;
            }
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
        symbolize,
    })
}

pub(super) fn parse_hex_u64(s: &str) -> Option<u64> {
    let trimmed = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(trimmed, 16).ok()
}

#[cfg(test)]
#[path = "tests/args_tests.rs"]
mod tests;
