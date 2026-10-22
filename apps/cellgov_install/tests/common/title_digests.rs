//! The committed title-digest manifest, `tests/parity_digests.toml`.
//!
//! Parsing and well-formedness live here rather than in the parity
//! suite so a fresh checkout sees a malformed manifest: the suite that
//! consumes these rows is gated on `installed-title-tests`, and the rows it does
//! not compare on this host would otherwise never be looked at.

#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use serde::Deserialize;

#[derive(Deserialize)]
struct DigestManifest {
    title: Vec<TitleDigest>,
}

/// One `[[title]]` row.
#[derive(Deserialize)]
pub struct TitleDigest {
    /// PS3 content id; also the directory the installer writes under.
    pub content_id: String,
    /// Human-readable title, used in diagnostics.
    pub display: String,
    /// Which install shape the row describes: `npdrm` or `app`.
    pub key: String,
    /// RAP basename under `exdata/`; `npdrm` rows only.
    pub rap_filename: Option<String>,
    /// SHA-256 of the decrypted ELF, unmasked.
    pub unmasked_sha256: String,
    /// SHA-256 after non-semantic ELF bytes are masked; `npdrm` only.
    pub masked_sha256: Option<String>,
}

impl TitleDigest {
    /// Whether the row's expected hashes come from RPCS3.
    ///
    /// `app` rows carry a CellGov-derived refactor-invariance baseline
    /// instead, so a run that compared only those held nothing against
    /// the reference implementation.
    pub fn is_rpcs3_reference(&self) -> bool {
        self.key == "npdrm"
    }
}

/// Path of the committed manifest.
pub fn manifest_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity_digests.toml")
}

/// Parse the committed manifest.
///
/// # Panics
///
/// If the file is unreadable or does not parse.
pub fn load() -> Vec<TitleDigest> {
    let path = manifest_path();
    let s =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse(&s, &path.display().to_string())
}

/// Parse `text`; `origin` names it in diagnostics.
///
/// # Panics
///
/// If `text` does not parse as the manifest schema.
pub fn parse(text: &str, origin: &str) -> Vec<TitleDigest> {
    let parsed: DigestManifest =
        toml::from_str(text).unwrap_or_else(|e| panic!("parse {origin}: {e}"));
    parsed.title
}

/// Decode a 64-character hex string; `ctx` names it in diagnostics.
///
/// # Panics
///
/// If the string is not exactly 64 hex characters.
pub fn hex_to_bytes32(s: &str, ctx: &str) -> [u8; 32] {
    assert_eq!(s.len(), 64, "{ctx}: hex must be 64 chars, got {}", s.len());
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| panic!("{ctx}: invalid hex byte at index {i} in {s:?}"));
    }
    out
}

/// Every row is well formed, not only the rows a given host compares.
///
/// # Panics
///
/// If the list is empty, a content id repeats, a key is unknown, a row
/// is missing a column its key requires, carries one its key ignores,
/// or holds a hash that is not 64 hex characters.
pub fn assert_well_formed(titles: &[TitleDigest]) {
    assert!(
        !titles.is_empty(),
        "parity_digests.toml must declare at least one [[title]] entry"
    );
    let mut seen: Vec<&str> = Vec::new();
    for entry in titles {
        let id = entry.content_id.as_str();
        assert!(
            !seen.contains(&id),
            "parity_digests.toml lists content_id {id:?} more than once; \
             delete the stale row rather than comparing the title twice"
        );
        seen.push(id);
        let title = &entry.display;
        let unmasked = hex_to_bytes32(&entry.unmasked_sha256, &format!("{title} unmasked"));
        match entry.key.as_str() {
            "npdrm" => {
                assert!(
                    entry.rap_filename.is_some(),
                    "{title}: npdrm row requires rap_filename in parity_digests.toml"
                );
                let masked_hex = entry.masked_sha256.as_ref().unwrap_or_else(|| {
                    panic!("{title}: npdrm row requires masked_sha256 in parity_digests.toml")
                });
                let masked = hex_to_bytes32(masked_hex, &format!("{title} masked"));
                assert_ne!(
                    masked, unmasked,
                    "{title}: masked_sha256 equals unmasked_sha256, so the \
                     masked fallback would accept exactly what the unmasked \
                     check already accepted"
                );
            }
            "app" => {
                assert!(
                    entry.masked_sha256.is_none(),
                    "{title}: app rows are compared unmasked, so a \
                     masked_sha256 here is never read; drop it or change \
                     the row's key"
                );
                assert!(
                    entry.rap_filename.is_none(),
                    "{title}: app rows install no RAP, so a rap_filename \
                     here is never read"
                );
            }
            other => panic!("{id}: unknown key {other:?} in parity_digests.toml"),
        }
    }
}
