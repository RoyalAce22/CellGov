/// Hard cap on `--count`. PPC instructions are 4 bytes, so 1<<16 lines
/// covers a 256 KB code region -- larger than any function-sized
/// investigation this tool exists to support.
pub(super) const MAX_COUNT: usize = 1 << 16;

pub(super) fn usage() -> &'static str {
    "usage: cellgov_cli disasm <elf-path> --vaddr <hex> [--count N]\n\
     \t--vaddr  hex address (with or without 0x prefix); must be 4-byte aligned\n\
     \t--count  decimal instruction count, 1..=65536, default 16"
}

#[derive(Debug)]
pub(super) struct DisasmArgs<'a> {
    pub(super) elf_path: &'a str,
    pub(super) vaddr: u64,
    pub(super) count: usize,
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

pub(super) fn parse_hex_u64(s: &str) -> Option<u64> {
    let trimmed = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(trimmed, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parse_args_accepts_count_at_max() {
        let argv = args_vec(&["--vaddr", "0x10000", "--count", "65536"]);
        let p = parse_args(&argv).unwrap();
        assert_eq!(p.count, MAX_COUNT);
    }
}
