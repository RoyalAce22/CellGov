//! Numeric-formatting helpers shared by the matrix renderers.

/// Render a `u64` as a decimal integer with `,` separators every
/// three digits.
///
/// # Examples
///
/// ```
/// use cellgov_compare::format::format_with_commas;
/// assert_eq!(format_with_commas(999), "999");
/// assert_eq!(format_with_commas(1_000), "1,000");
/// assert_eq!(format_with_commas(3_674_262_528), "3,674,262,528");
/// ```
pub fn format_with_commas(n: u64) -> String {
    let digits = n.to_string();
    let head_len = match digits.len() % 3 {
        0 => 3,
        r => r,
    };
    let mut out = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    out.push_str(&digits[..head_len]);
    let mut i = head_len;
    while i < digits.len() {
        out.push(',');
        out.push_str(&digits[i..i + 3]);
        i += 3;
    }
    out
}

#[cfg(test)]
#[path = "tests/format_tests.rs"]
mod tests;
