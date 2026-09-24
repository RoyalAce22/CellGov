//! Holds a set of acquired PUP files, and the installed firmware entries
//! they match, against an archive of known PUPs.
//!
//! The archive lives outside this crate, so the caller hands its rows
//! over as [`ArchivePup`] values. The caller also reads the files; the
//! verifier sees their bytes and the path to name them by.

use std::collections::{BTreeMap, BTreeSet};

use crate::manifest::{sha256_of, Sha256};
use crate::pup::{self, PupError};

/// One PUP the archive knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePup {
    /// SHA-256 over the whole file, lowercase hex.
    pub pup_sha256: String,
    /// The firmware version key the PUP installs.
    pub fw: String,
    /// The file's length in bytes.
    pub size_bytes: u64,
    /// The PUP header's image version, as `0x` and 16 hex digits.
    pub image_version: String,
}

/// What one acquired PUP file names itself as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PupIdentity {
    /// The firmware version key the PUP installs.
    pub fw: String,
    /// The PUP header's image version, as `0x` and 16 hex digits.
    pub image_version: String,
}

/// One acquired PUP file, hashed and parsed.
#[derive(Debug)]
pub struct ScannedPup {
    /// The name a report gives the file.
    pub path: String,
    /// SHA-256 over the file, lowercase hex.
    pub sha256: String,
    /// The file's length in bytes.
    pub size_bytes: u64,
    /// What the file names itself as, or why it names nothing.
    pub identity: Result<PupIdentity, PupError>,
}

impl ScannedPup {
    /// Hashes and parses `bytes`, the content of the file named `path`.
    #[must_use]
    pub fn of(path: String, bytes: &[u8]) -> Self {
        let identity = pup::parse(bytes).and_then(|parsed| {
            Ok(PupIdentity {
                fw: pup::version_key(bytes, &parsed)?,
                image_version: format!("0x{:016x}", parsed.image_version),
            })
        });
        Self {
            path,
            sha256: Sha256(sha256_of(bytes)).to_hex(),
            size_bytes: bytes.len() as u64,
            identity,
        }
    }
}

/// Which way a file or an installed entry disagrees with the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PupMismatchKind {
    /// The file's hash is known, but its version, size or image version
    /// is not the archive's.
    Metadata,
    /// The file does not parse as a PUP.
    InvalidPup,
    /// The file parses, and the archive knows no PUP with its hash.
    Sha256,
    /// An installed entry's version is not the archive row's.
    FirmwareVersion,
    /// An installed entry's image version is not the archive row's.
    ImageVersion,
    /// An installed entry's record names another source than its
    /// manifest does.
    SourceSha256,
    /// An installed entry's manifest names another version than the
    /// archive row.
    ManifestVersion,
}

impl PupMismatchKind {
    /// The name a report gives the kind.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::InvalidPup => "invalid-pup",
            Self::Sha256 => "sha256",
            Self::FirmwareVersion => "firmware-version",
            Self::ImageVersion => "image-version",
            Self::SourceSha256 => "source-sha256",
            Self::ManifestVersion => "manifest-version",
        }
    }
}

/// One disagreement with the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PupMismatch {
    /// The file or installed entry that disagrees.
    pub subject: String,
    /// Which way it disagrees.
    pub kind: PupMismatchKind,
    /// The firmware version the subject names, when it names one.
    pub fw: Option<String>,
    /// What the archive holds, as the report names it; empty when the
    /// archive holds no row for the subject.
    pub expected: Vec<String>,
    /// What the subject holds, as the report names it.
    pub found: Option<String>,
    /// Why the subject names nothing, for a file that does not parse.
    pub reason: Option<String>,
}

/// A set of acquired files sorted against the archive.
#[derive(Debug)]
pub struct PupClassification<'a> {
    /// Archive rows whose first file with their hash matches, with that
    /// file's path, ordered by hash.
    pub present: Vec<(&'a ArchivePup, String)>,
    /// Archive rows whose hash no file carries, in archive order.
    pub missing: Vec<&'a ArchivePup>,
    /// Files that disagree with the archive, ordered by subject.
    pub mismatched: Vec<PupMismatch>,
}

/// Sorts `scanned` against `rows` into present, missing and mismatched.
///
/// A file whose hash the archive knows is present when its version,
/// size and image version match too. Only the first file with a hash
/// counts as its row's copy.
#[must_use]
pub fn classify<'a>(rows: &'a [ArchivePup], scanned: &[ScannedPup]) -> PupClassification<'a> {
    let by_hash: BTreeMap<&str, &ArchivePup> = rows
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row))
        .collect();
    let mut by_fw: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for row in rows {
        by_fw
            .entry(row.fw.as_str())
            .or_default()
            .push(row.pup_sha256.as_str());
    }
    let mut seen = BTreeSet::new();
    let mut present = Vec::new();
    let mut mismatched = Vec::new();
    for found in scanned {
        if let Some(row) = by_hash.get(found.sha256.as_str()) {
            let first = seen.insert(row.pup_sha256.as_str());
            match &found.identity {
                Ok(id)
                    if id.fw == row.fw
                        && id.image_version == row.image_version
                        && found.size_bytes == row.size_bytes =>
                {
                    if first {
                        present.push((*row, found.path.clone()));
                    }
                }
                Ok(id) => mismatched.push(PupMismatch {
                    subject: found.path.clone(),
                    kind: PupMismatchKind::Metadata,
                    fw: Some(id.fw.clone()),
                    expected: vec![format!(
                        "fw {}, size {}, image {}",
                        row.fw, row.size_bytes, row.image_version
                    )],
                    found: Some(format!(
                        "fw {}, size {}, image {}",
                        id.fw, found.size_bytes, id.image_version
                    )),
                    reason: None,
                }),
                Err(error) => mismatched.push(PupMismatch {
                    subject: found.path.clone(),
                    kind: PupMismatchKind::InvalidPup,
                    fw: None,
                    expected: vec![row.pup_sha256.clone()],
                    found: Some(found.sha256.clone()),
                    reason: Some(error.to_string()),
                }),
            }
            continue;
        }
        match &found.identity {
            Ok(id) => mismatched.push(PupMismatch {
                subject: found.path.clone(),
                kind: PupMismatchKind::Sha256,
                fw: Some(id.fw.clone()),
                expected: by_fw.get(id.fw.as_str()).map_or_else(Vec::new, |hashes| {
                    hashes.iter().map(|hash| (*hash).to_string()).collect()
                }),
                found: Some(found.sha256.clone()),
                reason: None,
            }),
            Err(error) => mismatched.push(PupMismatch {
                subject: found.path.clone(),
                kind: PupMismatchKind::InvalidPup,
                fw: None,
                expected: Vec::new(),
                found: Some(found.sha256.clone()),
                reason: Some(error.to_string()),
            }),
        }
    }
    let missing = rows
        .iter()
        .filter(|row| !seen.contains(row.pup_sha256.as_str()))
        .collect();
    present.sort_by(|a, b| a.0.pup_sha256.cmp(&b.0.pup_sha256));
    mismatched.sort_by(|a, b| a.subject.cmp(&b.subject));
    PupClassification {
        present,
        missing,
        mismatched,
    }
}

/// What an installed firmware entry claims about the PUP it came from.
#[derive(Debug, Clone, Copy)]
pub struct InstalledClaims<'a> {
    /// The version the store files the entry under.
    pub version: &'a str,
    /// The source digest the entry's install record names.
    pub record_sha256: &'a str,
    /// The version the entry's `firmware.toml` names.
    pub manifest_version: &'a str,
    /// The source digest the entry's `firmware.toml` names.
    pub manifest_sha256: &'a str,
    /// The image version the entry's `firmware.toml` names.
    pub manifest_image_version: &'a str,
}

/// Every way an installed entry disagrees with the archive row of the
/// PUP its manifest names: its manifest's version, then its own
/// version, image version and source digest.
#[must_use]
pub fn installed_mismatches(claims: &InstalledClaims<'_>, row: &ArchivePup) -> Vec<PupMismatch> {
    let subject = format!("installed firmware {}", claims.version);
    let mismatch = |kind, fw: &str, expected: &str, found: &str| PupMismatch {
        subject: subject.clone(),
        kind,
        fw: Some(fw.to_string()),
        expected: vec![expected.to_string()],
        found: Some(found.to_string()),
        reason: None,
    };
    let mut out = Vec::new();
    // The boot identity gate rejects this same stale-manifest state in
    // `cellgov_boot::compose`.
    if claims.manifest_version != row.fw {
        out.push(mismatch(
            PupMismatchKind::ManifestVersion,
            claims.manifest_version,
            &row.fw,
            claims.manifest_version,
        ));
    }
    if claims.version != row.fw {
        out.push(mismatch(
            PupMismatchKind::FirmwareVersion,
            claims.version,
            &row.fw,
            claims.version,
        ));
    }
    if claims.manifest_image_version != row.image_version {
        out.push(mismatch(
            PupMismatchKind::ImageVersion,
            claims.version,
            &row.image_version,
            claims.manifest_image_version,
        ));
    }
    if claims.record_sha256 != claims.manifest_sha256 {
        out.push(mismatch(
            PupMismatchKind::SourceSha256,
            claims.version,
            &row.pup_sha256,
            claims.record_sha256,
        ));
    }
    out
}

#[cfg(test)]
#[path = "tests/pup_verify_tests.rs"]
mod tests;
