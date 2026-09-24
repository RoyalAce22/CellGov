//! The stateless line parsers.

use crate::keys::hex::{decode_hex, is_hex_token};
use crate::keys::names::{normalized, parse_revision, Kind};
use crate::keys::{Lv2Versions, SelfClass};

pub(super) fn strip_comment(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with('#') || t.starts_with(';') || t.starts_with("//") {
        return "";
    }
    // A trailing comment is only one that is set off by whitespace,
    // so a `sc_key::x` style name keeps its `::`.
    for marker in [" #", "\t#", " //", "\t//"] {
        if let Some(i) = line.find(marker) {
            return &line[..i];
        }
    }
    line
}

/// Quoted hexadecimal values in a constructor record, in source order.
pub(super) fn quoted_hex_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = line;
    while let Some((_, after_open)) = rest.split_once('"') {
        let Some((value, after_close)) = after_open.split_once('"') else {
            break;
        };
        if is_hex_token(value, value.len() / 2) && value.len() >= 0x20 {
            values.push(value.to_string());
        }
        rest = after_close;
    }
    values
}

/// The first two hexadecimal constructor arguments are a SELF key's
/// inclusive firmware-version range.
pub(super) fn constructor_version_range(line: &str) -> Option<(u64, u64)> {
    let before_strings = line.split('"').next()?;
    let values: Vec<u64> = before_strings
        .split(|c: char| !c.is_ascii_hexdigit() && c != 'x' && c != 'X')
        .filter_map(|token| {
            token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
        })
        .filter_map(|hex| u64::from_str_radix(hex, 16).ok())
        .collect();
    Some((*values.first()?, *values.get(1)?))
}

/// `prop=value` inside a `[section]`; the property is a bare word.
pub(super) fn split_property(line: &str) -> Option<(&str, &str)> {
    let (prop, value) = line.split_once('=')?;
    let prop = prop.trim();
    if prop.is_empty() || !prop.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((prop, value.trim()))
}

/// `name: HEX` / `name = HEX`: the last token is the value, everything
/// before the separator is the name. A value that is itself
/// colon-separated (`name: aa:bb:...`) is read from the first
/// separator instead, since the last one sits inside the value.
pub(super) fn named_value(line: &str) -> Option<(&str, &str)> {
    [
        line.rsplit_once('='),
        line.rsplit_once(':'),
        line.split_once('='),
        line.split_once(':'),
    ]
    .into_iter()
    .flatten()
    .find_map(|(name, value)| {
        let value = value.trim();
        let name = name.trim().trim_end_matches([':', '=']).trim();
        if value.is_empty() || !name.chars().any(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        decode_hex(value)
            .ok()
            .filter(|b| b.len() >= 16)
            .map(|_| (name, value))
    })
}

/// A line that is one hex value of at least 16 bytes and nothing else.
pub(super) fn bare_value(line: &str) -> Option<Vec<u8>> {
    let candidate = line.trim();
    let plausible = candidate.bytes().all(|b| {
        b.is_ascii_hexdigit()
            || matches!(
                b,
                b' ' | b'\t' | b',' | b':' | b'x' | b'X' | b'{' | b'}' | b';'
            )
    });
    if !plausible {
        return None;
    }
    decode_hex(candidate).ok().filter(|b| b.len() >= 16)
}

/// One pasted key-table row: a 32-byte ERK token immediately followed
/// by a 16-byte RIV token, with the class read off the other columns.
pub(super) struct TableRow {
    pub(super) kind: Option<Kind>,
    /// `0x..` as written (`sd-0x..` for a row badged `SD`, which files
    /// as a candidate beside the plain row at the same revision), or the
    /// version range of an LV2 row.
    pub(super) label: Option<String>,
    pub(super) first: String,
    pub(super) erk: [u8; 0x20],
    pub(super) riv: [u8; 0x10],
}

pub(super) fn table_row(line: &str) -> Option<TableRow> {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 3 {
        return None;
    }
    let erk_index = tokens.iter().position(|t| is_hex_token(t, 0x20))?;
    let riv_token = tokens.get(erk_index + 1)?;
    if !is_hex_token(riv_token, 0x10) {
        return None;
    }
    let erk = decode_hex(tokens[erk_index]).ok()?;
    let riv = decode_hex(riv_token).ok()?;
    let mut kind = None;
    let mut revision = None;
    let mut versions = None;
    let mut np_marker = false;
    let mut sd_marker = false;
    for (i, t) in tokens.iter().enumerate() {
        if i == erk_index || i == erk_index + 1 {
            continue;
        }
        match normalized(t).as_str() {
            "app" | "appldr" => kind = kind.or(Some(Kind::Class(SelfClass::App))),
            "npdrm" => kind = kind.or(Some(Kind::Class(SelfClass::Npdrm))),
            // `lv2ldr` rows stay unplaced; see `names::classify_name`.
            "lv2" => kind = kind.or(Some(Kind::Class(SelfClass::Lv2))),
            "pkg" | "spkg" | "scepkg" => kind = kind.or(Some(Kind::Scepkg)),
            "np" => np_marker = true,
            "sd" => sd_marker = true,
            _ => {}
        }
        // The revision column precedes the ERK; the `0x..` after the
        // RIV is the curve type, which would otherwise label the row.
        if i < erk_index && revision.is_none() && t.starts_with("0x") {
            revision = parse_revision(t);
        }
        // The version-range column (`3.60-3.61`) labels an LV2 row.
        if i < erk_index && versions.is_none() && t.contains('.') {
            versions = Lv2Versions::parse(t);
        }
    }
    if np_marker && matches!(kind, None | Some(Kind::Class(SelfClass::App))) {
        kind = Some(Kind::Class(SelfClass::Npdrm));
    }
    let label = match kind {
        Some(Kind::Class(SelfClass::Lv2)) => versions.map(|v| v.to_string()),
        _ => revision.map(|r| {
            if sd_marker {
                format!("sd-0x{r:04x}")
            } else {
                format!("0x{r:04x}")
            }
        }),
    };
    Some(TableRow {
        kind,
        label,
        first: tokens[0].to_string(),
        erk: erk.as_slice().try_into().ok()?,
        riv: riv.as_slice().try_into().ok()?,
    })
}
