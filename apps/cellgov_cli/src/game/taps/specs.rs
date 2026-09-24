//! The three watches' specs, read from the values of their variables.
//!
//! - HLE return watch: `CELLGOV_HLE_RETURN_WATCH` (comma-separated hex
//!   NIDs), `CELLGOV_HLE_RETURN_WATCH_PCS` (comma-separated `pc=name`
//!   for entries whose NID is not unique across PRXes) and
//!   `CELLGOV_HLE_RETURN_WATCH_PATH`.
//! - Store watch: `CELLGOV_STORE_WATCH=<addr>:<len>` and
//!   `CELLGOV_STORE_WATCH_PATH`.
//! - Value sample: `CELLGOV_VALUE_SAMPLE=<addr>:<width>` in hex (e.g.
//!   `0x91FE9C:4`), `CELLGOV_VALUE_SAMPLE_PATH`, and the optional
//!   decimal `CELLGOV_VALUE_SAMPLE_STRIDE` (default 1, every step; zero
//!   is out of range).
//!
//! An empty value reads as unset.

use std::path::PathBuf;

use cellgov_boot::taps::hle_watch::{MAX_NAME_LEN, RAW_PC_ID_BIT};
use cellgov_boot::taps::store_watch::{MAX_LEN, WINDOW_END};
use cellgov_boot::taps::value_sample::MAX_WIDTH;
use cellgov_boot::taps::{HleWatchSpec, StoreWatchSpec, ValueSampleSpec};

use super::error::TapError;
use super::parse::{hex_pair, hex_u32};
use crate::env_vars;

const WATCH_VARS: &str = "CELLGOV_HLE_RETURN_WATCH or CELLGOV_HLE_RETURN_WATCH_PCS";

/// The non-empty comma-separated tokens of `value`.
fn tokens(value: Option<&str>) -> impl Iterator<Item = &str> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// The HLE return watch's spec; `None` when no watch and no path is
/// set.
///
/// # Errors
///
/// Returns a [`TapError`] when:
///
/// - a number or a `pc=name` pair does not parse
/// - a raw-PC name is longer than 255 bytes
/// - a raw-PC ID equals a NID or the ID of another raw PC
/// - a watch is set without a path, or a path without a watch
pub(super) fn parse_hle(
    nids: Option<&str>,
    pcs: Option<&str>,
    path: Option<&str>,
) -> Result<Option<HleWatchSpec>, TapError> {
    let path = path.unwrap_or_default();
    let mut spec = HleWatchSpec {
        nids: Vec::new(),
        raw_pcs: Vec::new(),
        path: PathBuf::from(path),
    };
    for tok in tokens(nids) {
        spec.nids.push(hex_u32(env_vars::HLE_RETURN_WATCH, tok)?);
    }
    for tok in tokens(pcs) {
        let (pc, name) = tok.split_once('=').ok_or_else(|| TapError::BadShape {
            var: env_vars::HLE_RETURN_WATCH_PCS,
            expected: "<pc>=<name>",
            got: tok.to_string(),
        })?;
        let pc = hex_u32(env_vars::HLE_RETURN_WATCH_PCS, pc.trim())?;
        // The resolution record carries the name behind a 1-byte
        // length.
        if name.len() > MAX_NAME_LEN {
            return Err(TapError::BadShape {
                var: env_vars::HLE_RETURN_WATCH_PCS,
                expected: "<pc>=<name>, the name at most 255 bytes",
                got: tok.to_string(),
            });
        }
        let id = pc | RAW_PC_ID_BIT;
        if spec.nids.contains(&id) {
            return Err(TapError::RawPcCollides { pc, id });
        }
        // The ID sets bit 31, so a repeated PC, or two PCs that
        // differ in bit 31 alone, name one watch on the wire.
        if let Some(&(first, _)) = spec.raw_pcs.iter().find(|(p, _)| p | RAW_PC_ID_BIT == id) {
            return Err(TapError::RawPcsCollide {
                first,
                second: pc,
                id,
            });
        }
        spec.raw_pcs.push((pc, name.to_string()));
    }
    let watches = !(spec.nids.is_empty() && spec.raw_pcs.is_empty());
    match (watches, path.is_empty()) {
        (false, true) => Ok(None),
        (true, false) => Ok(Some(spec)),
        (true, true) => Err(TapError::Unpaired {
            set: WATCH_VARS,
            missing: env_vars::HLE_RETURN_WATCH_PATH,
        }),
        (false, false) => Err(TapError::Unpaired {
            set: env_vars::HLE_RETURN_WATCH_PATH,
            missing: WATCH_VARS,
        }),
    }
}

/// The store watch's spec; `None` when neither variable is set.
///
/// # Errors
///
/// Returns a [`TapError`] when:
///
/// - the window does not parse or is out of range
/// - one variable is set without the other
pub(super) fn parse_store(
    spec: Option<&str>,
    path: Option<&str>,
) -> Result<Option<StoreWatchSpec>, TapError> {
    let spec = spec.unwrap_or_default().trim();
    let path = path.unwrap_or_default();
    match (spec.is_empty(), path.is_empty()) {
        (true, true) => return Ok(None),
        (false, true) => {
            return Err(TapError::Unpaired {
                set: env_vars::STORE_WATCH,
                missing: env_vars::STORE_WATCH_PATH,
            })
        }
        (true, false) => {
            return Err(TapError::Unpaired {
                set: env_vars::STORE_WATCH_PATH,
                missing: env_vars::STORE_WATCH,
            })
        }
        (false, false) => {}
    }
    let (addr, len) = hex_pair(env_vars::STORE_WATCH, spec, "<addr>:<len>")?;
    if len == 0 || len > MAX_LEN {
        return Err(TapError::OutOfRange {
            var: env_vars::STORE_WATCH,
            value: len,
            range: "1..=0x10000",
        });
    }
    // The header's address and each record's `ea` are u32. For a
    // window past 4 GiB, the header names one address and the watch
    // covers another.
    if addr.saturating_add(len) > WINDOW_END {
        return Err(TapError::OutOfRange {
            var: env_vars::STORE_WATCH,
            value: addr,
            range: "a window ending at or below 0x1_0000_0000",
        });
    }
    Ok(Some(StoreWatchSpec {
        addr,
        len,
        path: PathBuf::from(path),
    }))
}

/// The value sample's spec; `None` when none of the variables is set.
///
/// # Errors
///
/// Returns a [`TapError`] when:
///
/// - the range or the stride does not parse or is out of range
/// - the range is set without the path, or the path without the range
/// - the stride is set without the range and the path
pub(super) fn parse_sample(
    spec: Option<&str>,
    path: Option<&str>,
    stride: Option<&str>,
) -> Result<Option<ValueSampleSpec>, TapError> {
    let spec = spec.unwrap_or_default().trim();
    let path = path.unwrap_or_default();
    let stride = stride.map(str::trim).filter(|s| !s.is_empty());
    match (spec.is_empty(), path.is_empty()) {
        (true, true) if stride.is_some() => {
            return Err(TapError::Unpaired {
                set: env_vars::VALUE_SAMPLE_STRIDE,
                missing: env_vars::VALUE_SAMPLE,
            })
        }
        (true, true) => return Ok(None),
        (false, true) => {
            return Err(TapError::Unpaired {
                set: env_vars::VALUE_SAMPLE,
                missing: env_vars::VALUE_SAMPLE_PATH,
            })
        }
        (true, false) => {
            return Err(TapError::Unpaired {
                set: env_vars::VALUE_SAMPLE_PATH,
                missing: env_vars::VALUE_SAMPLE,
            })
        }
        (false, false) => {}
    }
    let (addr, width) = hex_pair(env_vars::VALUE_SAMPLE, spec, "<addr>:<width>")?;
    if width == 0 || width > MAX_WIDTH {
        return Err(TapError::OutOfRange {
            var: env_vars::VALUE_SAMPLE,
            value: width,
            range: "1..=0x100",
        });
    }
    // The header's address field is u32. For a wider address, the
    // header names one address and the sample reads another.
    if addr > u64::from(u32::MAX) {
        return Err(TapError::OutOfRange {
            var: env_vars::VALUE_SAMPLE,
            value: addr,
            range: "32 bits",
        });
    }
    let stride = match stride {
        None => 1,
        Some(s) => s.parse::<u64>().map_err(|source| TapError::BadNumber {
            var: env_vars::VALUE_SAMPLE_STRIDE,
            token: s.to_string(),
            source,
        })?,
    };
    if stride == 0 {
        return Err(TapError::OutOfRange {
            var: env_vars::VALUE_SAMPLE_STRIDE,
            value: 0,
            range: "1..",
        });
    }
    Ok(Some(ValueSampleSpec {
        addr,
        width: width as u32,
        stride,
        path: PathBuf::from(path),
    }))
}

#[cfg(test)]
#[path = "tests/hle_spec_tests.rs"]
mod hle_spec_tests;

#[cfg(test)]
#[path = "tests/store_spec_tests.rs"]
mod store_spec_tests;

#[cfg(test)]
#[path = "tests/sample_spec_tests.rs"]
mod sample_spec_tests;
