//! The schema of the operator's NPDRM witness manifest,
//! `vfs/.cellgov/npdrm_oracle_vectors.toml`, and its parser.
//!
//! ```toml
//! [[vector]]
//! rap = "dev_hdd0/home/00000001/exdata/<content-id>.rap"  # relative to vfs/
//! klicensee = "<32 hex chars>"                            # what rap_to_klic derives
//! eboot = "dev_hdd0/game/<title-id>/USRDIR/EBOOT.BIN"      # optional; opens under it
//! ```
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::path::{Component, Path};

use serde::Deserialize;

/// The operator's manifest; unknown fields are refused so a misspelt
/// `eboot` cannot silently drop a row's container proof.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    vector: Vec<Vector>,
}

/// One `[[vector]]` row.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vector {
    /// RAP file, relative to the workspace `vfs/`.
    pub rap: String,
    /// Hex of the klicensee `rap_to_klic` derives from it.
    pub klicensee: String,
    /// NPDRM EBOOT that klicensee opens, relative to the workspace
    /// `vfs/`.
    pub eboot: Option<String>,
}

/// Parse `text`; `origin` names it in diagnostics.
///
/// # Panics
///
/// If `text` does not parse as the manifest schema, pins no vector,
/// pins the same RAP twice, pins a path that is not relative under
/// `vfs/`, or pins a klicensee that is not 32 hex characters.
pub fn parse(text: &str, origin: &str) -> Vec<Vector> {
    let parsed: Manifest = toml::from_str(text).unwrap_or_else(|e| panic!("parse {origin}: {e}"));
    assert!(
        !parsed.vector.is_empty(),
        "{origin} pins no [[vector]], so the suite would hold nothing"
    );
    let mut seen: Vec<&str> = Vec::new();
    for v in &parsed.vector {
        assert!(
            !seen.contains(&v.rap.as_str()),
            "{origin} pins RAP {:?} more than once; delete the stale row rather than \
             deriving it twice",
            v.rap
        );
        seen.push(v.rap.as_str());
        assert_under_vfs(&v.rap, "rap", &v.rap);
        if let Some(eboot) = &v.eboot {
            assert_under_vfs(eboot, "eboot", &v.rap);
        }
        hex_to_bytes16(&v.klicensee, &v.rap);
    }
    parsed.vector
}

/// Refuse a manifest path that is not a plain relative path under the
/// workspace `vfs/`; `ctx` names the row in diagnostics.
///
/// `vfs.join` replaces the base with an absolute path and follows
/// `..` out of it, so either would witness a file outside the
/// installed content the suite claims to hold.
///
/// # Panics
///
/// If `path` is empty, absolute, or climbs with `..`.
fn assert_under_vfs(path: &str, what: &str, ctx: &str) {
    let mut named = 0usize;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => named += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => panic!(
                "{ctx}: {what} {path:?} leaves the workspace vfs/; pin a path relative to it"
            ),
        }
    }
    assert!(
        named > 0,
        "{ctx}: {what} is empty; pin a path relative to the workspace vfs/"
    );
}

/// Decode a 32-character hex string; `ctx` names it in diagnostics.
///
/// # Panics
///
/// If the string is not exactly 32 hex characters.
pub fn hex_to_bytes16(s: &str, ctx: &str) -> [u8; 16] {
    assert_eq!(
        s.len(),
        32,
        "{ctx}: klicensee hex must be 32 chars, got {}",
        s.len()
    );
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| panic!("{ctx}: invalid hex byte at index {i} in {s:?}"));
    }
    out
}
