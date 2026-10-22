//! Committed RPCS3 reference digests, read by the parity tests.
//!
//! What RPCS3 produces for a given input is captured once into
//! `tests/fixtures/rpcs3_digests/digests.txt`, so a parity test needs
//! no RPCS3 install tree at run time.

// Each integration test binary compiles this module separately and
// uses a different subset of it.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Firmware module stems the table pins as `decrypted_masked/<stem>`
/// and `sprx/<version>/<stem>`.
pub const FIRMWARE_MODULES: &[&str] = &[
    "libaudio",
    "libfs",
    "libgcm_sys",
    "libio",
    "liblv2",
    "libnet",
    "libnetctl",
    "libspurs_jq",
    "libsync2",
    "libsysmodule",
    "libsysutil",
    "libsysutil_np",
];

/// Workspace root, resolved from this crate's manifest dir.
pub fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// One committed row: what RPCS3 produced for that key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Lowercase hex SHA-256 of the artifact.
    pub sha256: String,
    /// Byte length of the same artifact the digest covers.
    pub bytes: u64,
}

/// Parse the committed digest table, keyed as in the file.
///
/// # Panics
///
/// If the table is missing or a data line is malformed.
pub fn table() -> BTreeMap<String, Reference> {
    let path = workspace_root().join("tests/fixtures/rpcs3_digests/digests.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "committed reference digests unreadable at {}: {e}",
            path.display()
        )
    });
    parse_table(&text, &path.display().to_string())
}

/// Parse the digest table's `text`; `origin` names it in diagnostics.
///
/// # Panics
///
/// If a data line does not carry exactly the three documented columns,
/// its digest column is not 64 hex characters, its byte count is not a
/// number, or a key repeats.
pub fn parse_table(text: &str, origin: &str) -> BTreeMap<String, Reference> {
    const COLUMNS: &str = "columns are <sha256>  <key>  <bytes>";
    let mut out: BTreeMap<String, Reference> = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split_whitespace();
        let (Some(sha), Some(key), Some(bytes)) = (cols.next(), cols.next(), cols.next()) else {
            panic!(
                "{origin}:{}: malformed digest line {line:?}; {COLUMNS}",
                n + 1
            );
        };
        assert!(
            cols.next().is_none(),
            "{origin}:{}: digest line {line:?} carries a fourth column; {COLUMNS}",
            n + 1
        );
        assert!(
            sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "{origin}:{}: digest column {sha:?} is not 64 hex characters; {COLUMNS}",
            n + 1
        );
        let bytes: u64 = bytes.parse().unwrap_or_else(|e| {
            panic!(
                "{origin}:{}: byte-count column {bytes:?} is not a number: {e}; {COLUMNS}",
                n + 1
            )
        });
        // The comparison side formats lowercase hex; fold case here so
        // an uppercase row is a match rather than a phantom divergence.
        let sha = sha.to_ascii_lowercase();
        let row = Reference { sha256: sha, bytes };
        if let Some(prev) = out.insert(key.to_string(), row.clone()) {
            panic!(
                "{origin}:{}: key {key:?} appears more than once \
                 (first {}, then {}); delete the stale row rather \
                 than leaving insertion order to pick",
                n + 1,
                prev.sha256,
                row.sha256,
            );
        }
    }
    assert!(!out.is_empty(), "{origin} lists no digests");
    out
}

/// The firmware version the table's `sprx/<version>/<module>` rows name.
///
/// The plaintext rows come from those inputs, so the firmware parity
/// suite reads the install of this version.
///
/// # Panics
///
/// - No row is an `sprx/` row.
/// - An `sprx/` key does not split into a version and a module.
/// - Two rows name different versions.
pub fn reference_firmware(table: &BTreeMap<String, Reference>) -> String {
    let mut versions: BTreeSet<&str> = BTreeSet::new();
    for key in table.keys() {
        let Some(rest) = key.strip_prefix("sprx/") else {
            continue;
        };
        let Some((version, module)) = rest.split_once('/') else {
            panic!("digest key {key:?} does not name sprx/<version>/<module>");
        };
        assert!(
            !version.is_empty() && !module.is_empty() && !module.contains('/'),
            "digest key {key:?} does not name sprx/<version>/<module>"
        );
        versions.insert(version);
    }
    let mut iter = versions.iter();
    match (iter.next(), iter.next()) {
        (Some(version), None) => (*version).to_owned(),
        (None, _) => panic!(
            "the digest table has no sprx/<version>/<module> row, so no \
             firmware revision is recorded for its plaintext rows"
        ),
        (Some(_), Some(_)) => panic!(
            "the digest table's sprx/ rows name firmware versions {versions:?}; \
             its plaintext rows come from one install, so one version"
        ),
    }
}

/// Lowercase hex SHA-256 of `path`'s bytes.
pub fn sha256_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("read {} failed: {e}", path.display()));
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
