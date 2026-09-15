//! The number shapes the watch variables hold.

use super::error::TapError;

/// A hex number, with or without one `0x` prefix.
///
/// # Errors
///
/// Each refusal names `var`:
///
/// - [`TapError::BadShape`] for a signed number
/// - [`TapError::BadNumber`] for any other token that does not parse
pub(super) fn hex_u64(var: &'static str, token: &str) -> Result<u64, TapError> {
    let body = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    // `from_str_radix` accepts a leading `+`, and the patch set's hook
    // reads `0x0x10` as 0. This parser refuses both shapes.
    if body.starts_with(['+', '-']) {
        return Err(TapError::BadShape {
            var,
            expected: "an unsigned hex number",
            got: token.to_string(),
        });
    }
    u64::from_str_radix(body, 16).map_err(|source| TapError::BadNumber {
        var,
        token: token.to_string(),
        source,
    })
}

/// [`hex_u64`] for a number that fits 32 bits.
///
/// # Errors
///
/// Each refusal names `var`:
///
/// - any [`hex_u64`] refusal
/// - [`TapError::OutOfRange`] for a number wider than 32 bits
pub(super) fn hex_u32(var: &'static str, token: &str) -> Result<u32, TapError> {
    let value = hex_u64(var, token)?;
    u32::try_from(value).map_err(|_| TapError::OutOfRange {
        var,
        value,
        range: "32 bits",
    })
}

/// A `<hex>:<hex>` pair.
///
/// # Errors
///
/// Each refusal names `var`:
///
/// - [`TapError::BadShape`] for a value with no `:`
/// - the [`hex_u64`] refusal of either half
pub(super) fn hex_pair(
    var: &'static str,
    value: &str,
    expected: &'static str,
) -> Result<(u64, u64), TapError> {
    let (a, b) = value.split_once(':').ok_or_else(|| TapError::BadShape {
        var,
        expected,
        got: value.to_string(),
    })?;
    Ok((hex_u64(var, a.trim())?, hex_u64(var, b.trim())?))
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
