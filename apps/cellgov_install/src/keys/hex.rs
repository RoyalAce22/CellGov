//! Hex in the shapes keyfiles carry it: bare, `0x`-prefixed, C arrays,
//! colon- or dash-separated.

use super::HexError;

/// Decode hex, tolerating `0x` prefixes, whitespace, and the
/// separators C arrays and copied tables carry (`, : - _ { } ; " '`).
///
/// # Errors
///
/// [`HexError`] for a stray character or an odd digit count.
pub fn decode_hex(text: &str) -> Result<Vec<u8>, HexError> {
    let mut digits: Vec<u8> = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '0' if matches!(chars.peek(), Some('x' | 'X')) => {
                chars.next();
            }
            c if c.is_ascii_hexdigit() => digits.push(c as u8),
            c if c.is_whitespace()
                || matches!(c, ',' | ':' | '-' | '_' | '{' | '}' | ';' | '"' | '\'') => {}
            c => return Err(HexError::NonHex { ch: c }),
        }
    }
    if !digits.len().is_multiple_of(2) {
        return Err(HexError::OddLength {
            digits: digits.len(),
        });
    }
    Ok(digits
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
            let lo = (pair[1] as char).to_digit(16).unwrap_or(0) as u8;
            (hi << 4) | lo
        })
        .collect())
}

/// Lowercase hex of `bytes`, no separators.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `token` is exactly `bytes` bytes of contiguous hex (a `0x`
/// prefix allowed).
pub(super) fn is_hex_token(token: &str, bytes: usize) -> bool {
    let t = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    t.len() == bytes * 2 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A file's bytes as a key value: hex text when it decodes to a
/// key-sized value, the raw bytes when they are one, and otherwise
/// whichever reading is available.
///
/// Raw key bytes can all be ASCII hex digits, so the two readings are
/// told apart by which one lands on a key length.
pub(super) fn value_bytes(bytes: &[u8]) -> Vec<u8> {
    let key_sized = |len: usize| matches!(len, 0x10 | 0x20 | 0x40);
    let decoded = std::str::from_utf8(bytes)
        .ok()
        .filter(|t| t.bytes().any(|b| b.is_ascii_hexdigit()))
        .and_then(|t| decode_hex(t).ok());
    match decoded {
        Some(d) if key_sized(d.len()) => d,
        _ if key_sized(bytes.len()) => bytes.to_vec(),
        Some(d) => d,
        None => bytes.to_vec(),
    }
}
