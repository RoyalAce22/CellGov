//! Value parsers shared by the command tree: hex scalars, patch-byte
//! pairs, and fault-dump ranges.

/// Why a CLI argument or environment-variable parser rejected its input.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliArgError {
    #[error("{context}: empty hex value")]
    EmptyHexValue { context: String },
    #[error("{context}: hex prefix with no digits in {raw:?}")]
    HexPrefixNoDigits { context: String, raw: String },
    #[error("{context}: cannot parse hex {raw:?}: {source}")]
    CannotParseHexU64 {
        context: String,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("{context}: cannot parse hex u8 {raw:?}: {source}")]
    CannotParseHexU8 {
        context: String,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("{context}: cannot parse {raw:?} as a decimal number: {source}")]
    CannotParseDecimal {
        context: String,
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("expected 1-2 hex digits, got {digits} in {raw:?}")]
    HexU8TooLong { raw: String, digits: usize },
    #[error("{raw:?} does not fit a 32-bit address")]
    HexU32TooLarge { raw: String },
    #[error("empty entry in the comma list (leading, trailing or duplicate comma)")]
    EmptyCsvEntry,
    #[error("must be at least 1")]
    CountIsZero,
    #[error("{got} exceeds the maximum {max}")]
    CountTooLarge { got: usize, max: usize },
    #[error("empty argument (expected ADDR=VALUE)")]
    PatchByteEmpty,
    #[error("missing '=' in {pair:?} (expected ADDR=VALUE)")]
    PatchByteMissingEq { pair: String },
    #[error("empty address in {pair:?} (expected ADDR=VALUE)")]
    PatchByteEmptyAddress { pair: String },
    #[error("empty value in {pair:?} (expected ADDR=VALUE)")]
    PatchByteEmptyValue { pair: String },
    #[error("extra '=' in {pair:?} (expected ADDR=VALUE)")]
    PatchByteExtraEq { pair: String },
    #[error("extra ':' segment {rest:?} in {spec:?} (expected ADDR[:LEN])")]
    DumpRangeExtraColon { spec: String, rest: String },
    #[error("zero-byte length in {spec:?} (LEN must be > 0)")]
    DumpRangeZeroLen { spec: String },
    #[error("LEN 0x{len:x} exceeds maximum 0x{max:x} in {spec:?}")]
    DumpRangeTooLong { spec: String, len: u64, max: u64 },
    #[error("ADDR 0x{addr:016x} + LEN 0x{len:x} overflows u64 in {spec:?}")]
    DumpRangeOverflow { spec: String, addr: u64, len: u64 },
    #[error("{name}={got:?}: expected 0/1/true/false/yes/no/on/off")]
    EnvBoolUnknown { name: String, got: String },
}

/// Parse `s` as a hex u64 with optional `0x`/`0X` prefix.
pub(crate) fn parse_hex_u64_value(s: &str, context: &str) -> Result<u64, CliArgError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(CliArgError::EmptyHexValue {
            context: context.to_string(),
        });
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(CliArgError::HexPrefixNoDigits {
            context: context.to_string(),
            raw: s.to_string(),
        });
    }
    u64::from_str_radix(stripped, 16).map_err(|source| CliArgError::CannotParseHexU64 {
        context: context.to_string(),
        raw: s.to_string(),
        source,
    })
}

fn parse_hex_u8_value(s: &str, context: &str) -> Result<u8, CliArgError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(CliArgError::EmptyHexValue {
            context: context.to_string(),
        });
    }
    let stripped = strip_hex_prefix(trimmed);
    if stripped.is_empty() {
        return Err(CliArgError::HexPrefixNoDigits {
            context: context.to_string(),
            raw: s.to_string(),
        });
    }
    if stripped.len() > 2 {
        return Err(CliArgError::HexU8TooLong {
            raw: s.to_string(),
            digits: stripped.len(),
        });
    }
    u8::from_str_radix(stripped, 16).map_err(|source| CliArgError::CannotParseHexU8 {
        context: context.to_string(),
        raw: s.to_string(),
        source,
    })
}

pub(crate) fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// Parse one `ADDR=VALUE` pair, both hex.
pub(crate) fn parse_patch_byte_pair_value(pair: &str) -> Result<(u64, u8), CliArgError> {
    if pair.is_empty() {
        return Err(CliArgError::PatchByteEmpty);
    }
    let mut parts = pair.splitn(2, '=');
    let a_raw = parts
        .next()
        .expect("splitn(2) yields at least one element on a non-empty input");
    let b_raw = parts
        .next()
        .ok_or_else(|| CliArgError::PatchByteMissingEq {
            pair: pair.to_string(),
        })?;
    if a_raw.trim().is_empty() {
        return Err(CliArgError::PatchByteEmptyAddress {
            pair: pair.to_string(),
        });
    }
    if b_raw.trim().is_empty() {
        return Err(CliArgError::PatchByteEmptyValue {
            pair: pair.to_string(),
        });
    }
    if b_raw.contains('=') {
        return Err(CliArgError::PatchByteExtraEq {
            pair: pair.to_string(),
        });
    }
    let addr = parse_hex_u64_value(a_raw, "address")?;
    let val = parse_hex_u8_value(b_raw, "value")?;
    Ok((addr, val))
}

/// Sanity cap, in bytes, on a single fault-dump range.
const MAX_DUMP_LEN: u64 = 64 * 1024;

/// Default LEN when a fault-dump range names only an address.
const DEFAULT_DUMP_LEN: u64 = 0x40;

/// Parse `0xADDR` (default LEN) or `0xADDR:LEN`, both hex.
pub(crate) fn parse_dump_mem_fault_spec(spec: &str) -> Result<(u64, u64), CliArgError> {
    let mut parts = spec.splitn(3, ':');
    let addr_str = parts.next().unwrap_or("");
    let len_str = parts.next();
    if let Some(rest) = parts.next() {
        return Err(CliArgError::DumpRangeExtraColon {
            spec: spec.to_string(),
            rest: rest.to_string(),
        });
    }
    let addr = parse_hex_u64_value(addr_str, "address")?;
    let len = match len_str {
        Some(l) => parse_hex_u64_value(l, "length")?,
        None => DEFAULT_DUMP_LEN,
    };
    if len == 0 {
        return Err(CliArgError::DumpRangeZeroLen {
            spec: spec.to_string(),
        });
    }
    if len > MAX_DUMP_LEN {
        return Err(CliArgError::DumpRangeTooLong {
            spec: spec.to_string(),
            len,
            max: MAX_DUMP_LEN,
        });
    }
    if addr.checked_add(len - 1).is_none() {
        return Err(CliArgError::DumpRangeOverflow {
            spec: spec.to_string(),
            addr,
            len,
        });
    }
    Ok((addr, len))
}

#[cfg(test)]
#[path = "tests/args_tests.rs"]
mod tests;
