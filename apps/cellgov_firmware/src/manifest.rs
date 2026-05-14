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
/// chars; in-memory form is `[u8; 32]`. Custom serde keeps the type
/// wall intact: callers compare digests by byte equality, never by
/// string equality (which would silently mismatch on case, padding,
/// or `0x` prefix).
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

/// Top-level manifest. Field order matches the on-disk format so a
/// round-trip serialise / parse is stable. Construction outside this
/// module goes through `TryFrom<RawManifest>` so the
/// version/duplicate-path checks cannot be bypassed by a caller that
/// reaches for `toml::from_str::<FirmwareManifest>` directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawManifest")]
pub struct FirmwareManifest {
    pub format_version: u32,
    pub firmware: FirmwareIdentity,
    pub files: Vec<FirmwareFileEntry>,
}

/// Identifies the PUP a firmware install came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareIdentity {
    pub image_version: String,
    pub pup_sha256: Sha256,
}

/// Per-file integrity entry. `path` is relative to the install root
/// (e.g., `sys/external/liblv2.sprx`); `sha256` is over the
/// post-decrypt ELF bytes; `revision` is the SCE container header's
/// per-file revision tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareFileEntry {
    pub path: String,
    pub sha256: Sha256,
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

#[derive(Debug)]
pub enum ManifestError {
    UnsupportedFormatVersion {
        found: u32,
        expected: u32,
    },
    /// Two `[[files]]` entries shared the same `path`. Carries the
    /// offending path so the operator can locate the duplicate.
    DuplicatePath(String),
    /// Underlying TOML parse failure (malformed input, missing
    /// required field, type mismatch, malformed hex digest).
    Toml(toml::de::Error),
    /// Underlying TOML serialise failure.
    TomlSer(toml::ser::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::UnsupportedFormatVersion { found, expected } => {
                write!(
                    f,
                    "unsupported firmware.toml format_version {found} (expected {expected})"
                )
            }
            ManifestError::DuplicatePath(p) => {
                write!(f, "firmware.toml has duplicate [[files]].path: {p:?}")
            }
            ManifestError::Toml(e) => write!(f, "firmware.toml parse: {e}"),
            ManifestError::TomlSer(e) => write!(f, "firmware.toml serialise: {e}"),
        }
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(e: toml::de::Error) -> Self {
        ManifestError::Toml(e)
    }
}

impl From<toml::ser::Error> for ManifestError {
    fn from(e: toml::ser::Error) -> Self {
        ManifestError::TomlSer(e)
    }
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
        expected: [u8; 32],
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
#[derive(Debug, PartialEq, Eq)]
pub struct EmptyManifest;

impl std::fmt::Display for EmptyManifest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "firmware.toml has zero [[files]] entries; cannot verify vacuously"
        )
    }
}

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
    /// `Match`-verified. Otherwise returns the unmatched paths in
    /// manifest order.
    pub fn finish(self) -> Result<(), Vec<String>> {
        let unmatched: Vec<String> = self
            .manifest
            .files
            .iter()
            .zip(&self.matched)
            .filter(|(_, m)| !**m)
            .map(|(e, _)| e.path.clone())
            .collect();
        if unmatched.is_empty() {
            Ok(())
        } else {
            Err(unmatched)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(b: u8) -> Sha256 {
        Sha256([b; 32])
    }

    // Kept in sorted path order: `serialize_manifest` canonicalizes,
    // so the round-trip test would otherwise compare canonical-vs-
    // input and fail. New entries here must remain sorted.
    fn sample_manifest() -> FirmwareManifest {
        FirmwareManifest {
            format_version: SUPPORTED_FORMAT_VERSION,
            firmware: FirmwareIdentity {
                image_version: "4.85".into(),
                pup_sha256: sha(0x00),
            },
            files: vec![
                FirmwareFileEntry {
                    path: "sys/external/libfs.sprx".into(),
                    sha256: sha(0xAA),
                    revision: 0x1c,
                },
                FirmwareFileEntry {
                    path: "sys/external/liblv2.sprx".into(),
                    sha256: sha(0xBB),
                    revision: 0x1c,
                },
            ],
        }
    }

    #[test]
    fn round_trip_preserves_every_field() {
        let m = sample_manifest();
        let text = serialize_manifest(&m).expect("ser");
        let parsed = parse_manifest(&text).expect("parse");
        assert_eq!(parsed, m);
    }

    #[test]
    fn serialise_is_deterministic_across_two_calls() {
        let m = sample_manifest();
        let t1 = serialize_manifest(&m).expect("ser1");
        let t2 = serialize_manifest(&m).expect("ser2");
        assert_eq!(t1, t2);
    }

    #[test]
    fn serialise_sorts_files_by_path_regardless_of_input_order() {
        let mut m = sample_manifest();
        let unsorted = vec![m.files[1].clone(), m.files[0].clone()];
        m.files = unsorted;
        let t_unsorted = serialize_manifest(&m).expect("ser");

        let m_sorted = sample_manifest();
        let t_sorted = serialize_manifest(&m_sorted).expect("ser");

        assert_eq!(t_unsorted, t_sorted);
    }

    #[test]
    fn unsupported_format_version_errors_via_parse_manifest() {
        let mut m = sample_manifest();
        m.format_version = 2;
        // Bypass the try_from gate by serialising the raw form
        // through toml::to_string against a struct that mirrors
        // RawManifest's shape.
        #[derive(Serialize)]
        struct Forged<'a> {
            format_version: u32,
            firmware: &'a FirmwareIdentity,
            files: &'a [FirmwareFileEntry],
        }
        let text = toml::to_string(&Forged {
            format_version: m.format_version,
            firmware: &m.firmware,
            files: &m.files,
        })
        .expect("forge");
        let err = parse_manifest(&text).unwrap_err();
        let inner = match err {
            ManifestError::Toml(e) => e.to_string(),
            other => panic!("expected Toml-wrapped UnsupportedFormatVersion, got {other:?}"),
        };
        assert!(
            inner.contains("unsupported firmware.toml format_version 2"),
            "wrong inner message: {inner}"
        );
    }

    #[test]
    fn forged_future_version_via_direct_from_str_is_also_rejected() {
        // The try_from attribute makes the version check structural;
        // a caller bypassing parse_manifest and reaching for
        // toml::from_str directly gets the same rejection.
        let text = "format_version = 2\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"00000000000000000000000000000000000000000000000000000000000000ff\"\n";
        let err = toml::from_str::<FirmwareManifest>(text).unwrap_err();
        assert!(
            err.to_string().contains("unsupported"),
            "expected version rejection from direct toml::from_str, got: {err}"
        );
    }

    #[test]
    fn malformed_toml_surfaces_as_toml_error() {
        let err = parse_manifest("not [valid").unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn missing_required_field_surfaces_as_toml_error() {
        let text = "format_version = 1\n";
        let err = parse_manifest(text).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn duplicate_path_is_rejected_at_parse_time() {
        let dup = "ee".repeat(32);
        let text = format!(
            r#"format_version = 1

[firmware]
image_version = "x"
pup_sha256 = "{}"

[[files]]
path = "a.sprx"
sha256 = "{dup}"
revision = 0

[[files]]
path = "a.sprx"
sha256 = "{dup}"
revision = 0
"#,
            "00".repeat(32),
        );
        let err = parse_manifest(&text).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate") && msg.contains("a.sprx"),
            "expected duplicate-path rejection, got: {msg}"
        );
    }

    #[test]
    fn uppercase_hex_is_rejected_at_parse_time() {
        let text = format!(
            "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"{}\"\n",
            "AA".repeat(32),
        );
        let err = parse_manifest(&text).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn short_hex_is_rejected_at_parse_time() {
        let text =
            "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"deadbeef\"\n";
        let err = parse_manifest(text).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn non_hex_chars_are_rejected_at_parse_time() {
        let text = format!(
            "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"{}\"\n",
            "g0".repeat(32),
        );
        let err = parse_manifest(&text).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn verify_post_decrypt_match_returns_match() {
        let m = sample_manifest();
        let r = verify_post_decrypt(&m, "sys/external/libfs.sprx", &[0xAA; 32]);
        assert_eq!(r, VerifyOutcome::Match);
    }

    #[test]
    fn verify_post_decrypt_unknown_path_returns_not_in_manifest() {
        let m = sample_manifest();
        let r = verify_post_decrypt(&m, "sys/external/nope.sprx", &[0u8; 32]);
        assert_eq!(r, VerifyOutcome::NotInManifest);
    }

    #[test]
    fn verify_post_decrypt_wrong_digest_returns_mismatch() {
        let m = sample_manifest();
        let r = verify_post_decrypt(&m, "sys/external/libfs.sprx", &[0xFF; 32]);
        assert_eq!(
            r,
            VerifyOutcome::Mismatch {
                expected: [0xAA; 32],
                actual: [0xFF; 32],
            }
        );
    }

    #[test]
    fn files_table_can_be_empty() {
        // The cellgov_firmware install pipeline does not contractually
        // guarantee at least one decrypted SPRX -- a PUP without APP
        // keys we recognise produces zero entries, and the manifest
        // should still round-trip rather than refuse to serialise.
        let m = FirmwareManifest {
            format_version: SUPPORTED_FORMAT_VERSION,
            firmware: FirmwareIdentity {
                image_version: "1.00".into(),
                pup_sha256: sha(0x00),
            },
            files: Vec::new(),
        };
        let text = serialize_manifest(&m).expect("ser");
        let parsed = parse_manifest(&text).expect("parse");
        assert_eq!(parsed.files.len(), 0);
    }

    #[test]
    fn manifest_verifier_rejects_empty_manifest() {
        // Schema permits empty files; verifier does not. Empty
        // would finish() Ok against zero verifications, which is
        // the trivially-true case we are ruling out.
        let m = FirmwareManifest {
            format_version: SUPPORTED_FORMAT_VERSION,
            firmware: FirmwareIdentity {
                image_version: "1.00".into(),
                pup_sha256: sha(0x00),
            },
            files: Vec::new(),
        };
        assert_eq!(ManifestVerifier::new(&m).unwrap_err(), EmptyManifest);
    }

    #[test]
    fn manifest_verifier_finish_succeeds_when_every_entry_matched() {
        let m = sample_manifest();
        let mut v = ManifestVerifier::new(&m).expect("non-empty");
        assert_eq!(
            v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
            VerifyOutcome::Match
        );
        assert_eq!(
            v.verify_one("sys/external/liblv2.sprx", &[0xBB; 32]),
            VerifyOutcome::Match
        );
        assert!(v.finish().is_ok());
    }

    #[test]
    fn manifest_verifier_finish_returns_unmatched_paths_in_manifest_order() {
        let m = sample_manifest();
        let mut v = ManifestVerifier::new(&m).expect("non-empty");
        assert_eq!(
            v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
            VerifyOutcome::Match
        );
        let unmatched = v.finish().unwrap_err();
        assert_eq!(unmatched, vec!["sys/external/liblv2.sprx".to_string()]);
    }

    #[test]
    fn manifest_verifier_mismatch_does_not_count_as_matched() {
        let m = sample_manifest();
        let mut v = ManifestVerifier::new(&m).expect("non-empty");
        // Wrong digest: returns Mismatch and does NOT flip matched[].
        assert!(matches!(
            v.verify_one("sys/external/libfs.sprx", &[0xFF; 32]),
            VerifyOutcome::Mismatch { .. }
        ));
        assert_eq!(
            v.verify_one("sys/external/liblv2.sprx", &[0xBB; 32]),
            VerifyOutcome::Match
        );
        let unmatched = v.finish().unwrap_err();
        assert_eq!(unmatched, vec!["sys/external/libfs.sprx".to_string()]);
    }

    #[test]
    fn manifest_verifier_unknown_path_is_not_in_manifest_and_does_not_count() {
        let m = sample_manifest();
        let mut v = ManifestVerifier::new(&m).expect("non-empty");
        assert_eq!(
            v.verify_one("sys/external/nope.sprx", &[0u8; 32]),
            VerifyOutcome::NotInManifest
        );
        assert_eq!(
            v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
            VerifyOutcome::Match
        );
        let unmatched = v.finish().unwrap_err();
        assert_eq!(unmatched, vec!["sys/external/liblv2.sprx".to_string()]);
    }
}
