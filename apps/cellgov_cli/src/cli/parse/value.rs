//! House value parsers, used as clap `value_parser`s.

use crate::cli::args::{
    parse_dump_mem_fault_spec, parse_hex_u64_value, parse_patch_byte_pair_value, CliArgError,
};

/// A hex u64 with an optional `0x` / `0X` prefix.
pub(crate) fn hex_u64(s: &str) -> Result<u64, CliArgError> {
    parse_hex_u64_value(s, "value")
}

/// A hex u32 with an optional `0x` / `0X` prefix.
pub(crate) fn hex_u32(s: &str) -> Result<u32, CliArgError> {
    let wide = parse_hex_u64_value(s, "value")?;
    u32::try_from(wide).map_err(|_| CliArgError::HexU32TooLarge { raw: s.to_string() })
}

/// A step count, accepting the `0x` spelling the address flags take.
pub(crate) fn step_count(s: &str) -> Result<u64, CliArgError> {
    if s.starts_with("0x") || s.starts_with("0X") {
        return parse_hex_u64_value(s, "step");
    }
    s.parse().map_err(|source| CliArgError::CannotParseDecimal {
        context: "step".to_string(),
        raw: s.to_string(),
        source,
    })
}

/// One hex address of a comma list.
pub(crate) fn hex_addr(entry: &str) -> Result<u64, CliArgError> {
    reject_empty_entry(entry)?;
    parse_hex_u64_value(entry, "address")
}

/// One `ADDR[:LEN]` fault-dump range of a comma list.
pub(crate) fn dump_mem_fault_range(entry: &str) -> Result<(u64, u64), CliArgError> {
    reject_empty_entry(entry)?;
    parse_dump_mem_fault_spec(entry)
}

/// One `ADDR=VALUE` patch-byte pair of a comma list.
pub(crate) fn patch_byte_pair(entry: &str) -> Result<(u64, u8), CliArgError> {
    reject_empty_entry(entry)?;
    parse_patch_byte_pair_value(entry)
}

/// A boot stop condition: `process-exit`, `first-rsx-write`, or
/// `pc=0xADDR`.
pub(crate) fn checkpoint(
    value: &str,
) -> Result<crate::game::manifest::CheckpointTrigger, crate::game::manifest::CheckpointParseError> {
    crate::game::manifest::CheckpointTrigger::parse_cli_value(value)
}

/// Refuse an empty entry, which a leading, trailing or doubled comma
/// leaves behind.
///
/// clap splits the comma list on `value_delimiter` before a
/// `value_parser` sees it, so an empty entry arrives here as an empty
/// value.
fn reject_empty_entry(entry: &str) -> Result<(), CliArgError> {
    if entry.is_empty() {
        return Err(CliArgError::EmptyCsvEntry);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/value_tests.rs"]
mod tests;
