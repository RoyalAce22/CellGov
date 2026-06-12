//! `firmware.toml` schema. Produced by `cellgov_firmware install`
//! and consumed by `cellgov_cli`'s boot path to verify that the
//! installed firmware matches a known PUP revision before any title
//! boots through it.
//!
//! # Invariants enforced at parse time
//!
//! - `format_version` matches [`SUPPORTED_FORMAT_VERSION`].
//! - Every `sha256` field is 64 lowercase hex chars (32 bytes).
//! - `[[files]]` has no duplicate `path` entries.
//!
//! # Determinism
//!
//! [`serialize_manifest`] sorts `files` by `path` before emitting, so
//! the on-disk byte output is a pure function of the logical content,
//! independent of TAR-walk order or filesystem enumeration order on
//! the producer host.

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Schema version this build understands. A mismatch on parse fails
/// fast; the loader never tries to silently read a future schema.
pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

/// Fixed-width SHA-256 digest. On-disk form is 64 lowercase hex
/// chars; in-memory form is `[u8; 32]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    /// Lowercase hex string (64 chars, no prefix). Allocates.
    pub fn to_hex(&self) -> String {
        hex_encode_32(&self.0)
    }
}

impl Serialize for Sha256 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex_encode_32(&self.0))
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        parse_lowercase_hex_32(&s)
            .map(Sha256)
            .map_err(D::Error::custom)
    }
}

fn parse_lowercase_hex_32(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 {
        return Err(format!(
            "sha256 must be 64 lowercase hex chars; got {} chars",
            s.len()
        ));
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(
            "sha256 must be 64 lowercase hex chars; found a non-hex or uppercase byte".to_string(),
        );
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("sha256 byte {i}: {e}"))?;
    }
    Ok(out)
}

/// Top-level manifest. Construction goes through `TryFrom<RawManifest>`
/// so the version/duplicate-path checks cannot be bypassed by a
/// `toml::from_str::<FirmwareManifest>` caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawManifest")]
pub struct FirmwareManifest {
    /// Schema version; checked against [`SUPPORTED_FORMAT_VERSION`] at parse.
    pub format_version: u32,
    /// Which PUP this install came from.
    pub firmware: FirmwareIdentity,
    /// Per-file integrity entries. Serialised in sorted-by-`path` order.
    pub files: Vec<FirmwareFileEntry>,
}

/// Identifies the PUP a firmware install came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareIdentity {
    /// Human-readable image version string from the PUP (e.g., `"4.85"`).
    pub image_version: String,
    /// SHA-256 over the source PUP file bytes.
    pub pup_sha256: Sha256,
}

/// Per-file integrity entry. `path` is relative to the install root
/// (e.g., `sys/external/liblv2.sprx`); `sha256` is over the
/// post-decrypt ELF bytes; `revision` is the SCE container header's
/// per-file revision tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareFileEntry {
    /// Install-root-relative path (e.g., `sys/external/liblv2.sprx`).
    /// Unique across the manifest; duplicates are rejected at parse time.
    pub path: String,
    /// SHA-256 over the post-decrypt ELF bytes.
    pub sha256: Sha256,
    /// SCE container per-file revision tag the entry was decrypted under.
    pub revision: u16,
}

/// Raw on-disk shape. Deserialised first, then `TryFrom`-validated
/// into `FirmwareManifest`. The serde derive on `RawManifest` is the
/// only place TOML actually reads into; `FirmwareManifest`'s
/// `try_from` attribute makes the validation structurally
/// unavoidable.
#[derive(Deserialize)]
struct RawManifest {
    format_version: u32,
    firmware: FirmwareIdentity,
    #[serde(default)]
    files: Vec<FirmwareFileEntry>,
}

impl TryFrom<RawManifest> for FirmwareManifest {
    type Error = ManifestError;

    fn try_from(raw: RawManifest) -> Result<Self, Self::Error> {
        if raw.format_version != SUPPORTED_FORMAT_VERSION {
            return Err(ManifestError::UnsupportedFormatVersion {
                found: raw.format_version,
                expected: SUPPORTED_FORMAT_VERSION,
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in &raw.files {
            if !seen.insert(e.path.as_str()) {
                return Err(ManifestError::DuplicatePath(e.path.clone()));
            }
        }
        Ok(FirmwareManifest {
            format_version: raw.format_version,
            firmware: raw.firmware,
            files: raw.files,
        })
    }
}

/// Failure modes for parsing, serialising, or verifying a firmware manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// `format_version` field did not equal [`SUPPORTED_FORMAT_VERSION`].
    #[error("unsupported firmware.toml format_version {found} (expected {expected})")]
    UnsupportedFormatVersion {
        /// `format_version` value read from the manifest.
        found: u32,
        /// Schema version this build understands.
        expected: u32,
    },
    /// Two `[[files]]` entries shared the same `path`. Carries the
    /// offending path so the operator can locate the duplicate.
    #[error("firmware.toml has duplicate [[files]].path: {0:?}")]
    DuplicatePath(String),
    /// Underlying TOML parse failure (malformed input, missing
    /// required field, type mismatch, malformed hex digest).
    #[error("firmware.toml parse: {0}")]
    Toml(#[from] toml::de::Error),
    /// Underlying TOML serialise failure.
    #[error("firmware.toml serialise: {0}")]
    TomlSer(#[from] toml::ser::Error),
    /// Manifest entry was never verified by [`ManifestVerifier`]
    /// before `finish()`; the install pipeline either failed to
    /// produce the corresponding SPRX or never queued it for
    /// verification.
    #[error("firmware.toml entry {0:?} was never verified")]
    EntryUnverified(String),
}

/// Parse a `firmware.toml` text. Rejects unsupported
/// `format_version`s, duplicate `path` entries, and malformed hex
/// digests. The version + duplicate-path checks live inside
/// `TryFrom<RawManifest>` so direct `toml::from_str::<FirmwareManifest>`
/// callers get the same gates.
pub fn parse_manifest(text: &str) -> Result<FirmwareManifest, ManifestError> {
    toml::from_str(text).map_err(ManifestError::Toml)
}

/// Serialise `manifest` deterministically. `files` is sorted by
/// `path` before emitting so the byte output is a pure function of
/// the logical content; producers do not have to remember to sort.
pub fn serialize_manifest(manifest: &FirmwareManifest) -> Result<String, ManifestError> {
    let mut canon = manifest.clone();
    canon.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(toml::to_string(&canon)?)
}

/// Outcome of comparing a loaded PRX's post-decrypt bytes against the
/// manifest entry that names it.
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Hash matches the manifest entry.
    Match,
    /// `path` is not present in `manifest.files`.
    NotInManifest,
    /// Hash disagreed; carries both digests as bytes.
    Mismatch {
        /// Digest the manifest entry recorded.
        expected: [u8; 32],
        /// Digest computed over the loaded PRX's post-decrypt bytes.
        actual: [u8; 32],
    },
}

/// Hex-encode a 32-byte digest as lowercase ASCII. Local helper so
/// the manifest crate stays free of the `hex` dependency.
fn hex_encode_32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Verify a single loaded PRX against the manifest. `path` is the
/// install-root-relative path the loader supplies (matching the
/// manifest's `[[files]].path`); `post_decrypt_sha256` is the SHA-256
/// over the reconstructed ELF bytes.
///
/// Single-shot verifier; the boot path uses [`ManifestVerifier`] to
/// guarantee every manifest entry is verified at least once.
pub fn verify_post_decrypt(
    manifest: &FirmwareManifest,
    path: &str,
    post_decrypt_sha256: &[u8; 32],
) -> VerifyOutcome {
    let Some(entry) = manifest.files.iter().find(|f| f.path == path) else {
        return VerifyOutcome::NotInManifest;
    };
    if entry.sha256.0 == *post_decrypt_sha256 {
        VerifyOutcome::Match
    } else {
        VerifyOutcome::Mismatch {
            expected: entry.sha256.0,
            actual: *post_decrypt_sha256,
        }
    }
}

/// Constructor refused an empty manifest at the verifier boundary.
///
/// The schema itself permits zero `[[files]]` entries (a PUP whose
/// SPRXes all fail to decrypt for lack of APP keys legitimately
/// produces this), but a consumer that promised "verify every
/// manifest entry" cannot satisfy that promise vacuously over zero
/// entries. The boot path constructs the verifier through
/// [`ManifestVerifier::new`] and gets this error back instead of a
/// trivially-passing `finish()`.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("firmware.toml has zero [[files]] entries; cannot verify vacuously")]
pub struct EmptyManifest;

/// Aggregate verifier. Constructed once at boot, fed each loaded
/// PRX's `(path, digest)` pair, and drained with [`Self::finish`] --
/// which fails if any manifest entry went unverified. The boot path
/// cannot exit firmware setup without `finish`-ing, so a missing-PRX
/// bug becomes a hard error at boot rather than an invisible
/// footgun. Empty manifests are rejected at `new` time so the
/// vacuous-true case cannot pass `finish`.
#[derive(Debug)]
pub struct ManifestVerifier<'a> {
    manifest: &'a FirmwareManifest,
    matched: Vec<bool>,
}

impl<'a> ManifestVerifier<'a> {
    /// Construct a verifier. Returns [`EmptyManifest`] if the
    /// manifest has zero `[[files]]` entries -- a verifier built
    /// against zero entries would `finish()` `Ok(())` against zero
    /// verifications, which is exactly the trivially-true case the
    /// "verify firmware matches a known PUP" contract is meant to
    /// rule out.
    pub fn new(manifest: &'a FirmwareManifest) -> Result<Self, EmptyManifest> {
        if manifest.files.is_empty() {
            return Err(EmptyManifest);
        }
        Ok(Self {
            matched: vec![false; manifest.files.len()],
            manifest,
        })
    }

    /// Verify one `(path, digest)` pair. On `Match`, marks the
    /// corresponding manifest entry as seen; on `NotInManifest` /
    /// `Mismatch`, no entry is marked.
    pub fn verify_one(&mut self, path: &str, digest: &[u8; 32]) -> VerifyOutcome {
        let Some(idx) = self.manifest.files.iter().position(|f| f.path == path) else {
            return VerifyOutcome::NotInManifest;
        };
        let entry = &self.manifest.files[idx];
        if entry.sha256.0 == *digest {
            self.matched[idx] = true;
            VerifyOutcome::Match
        } else {
            VerifyOutcome::Mismatch {
                expected: entry.sha256.0,
                actual: *digest,
            }
        }
    }

    /// Returns `Ok(())` iff every manifest entry has been
    /// `Match`-verified. Otherwise returns one
    /// [`ManifestError::EntryUnverified`] per unmatched path, in
    /// manifest order.
    pub fn finish(self) -> Result<(), Vec<ManifestError>> {
        let unmatched: Vec<ManifestError> = self
            .manifest
            .files
            .iter()
            .zip(&self.matched)
            .filter(|(_, m)| !**m)
            .map(|(e, _)| ManifestError::EntryUnverified(e.path.clone()))
            .collect();
        if unmatched.is_empty() {
            Ok(())
        } else {
            Err(unmatched)
        }
    }
}

#[cfg(test)]
#[path = "tests/manifest_tests.rs"]
mod tests;
