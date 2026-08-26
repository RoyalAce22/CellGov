//! Checkpoint region manifest shared by every producer of a boot
//! observation: the CellGov `run-game --observation-manifest` path
//! and the RPCS3 dump bridge read the same file, so the regions they
//! capture pair by name.

use std::fmt;
use std::path::Path;

use serde::de::{self, Deserializer, Visitor};
use serde::Deserialize;

use crate::runner_cellgov::RegionDescriptor;
use crate::AddressSpaceId;

/// Region list in declaration order, which is also the order the
/// RPCS3 dump hook writes them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CheckpointManifest {
    /// Regions to capture.
    pub regions: Vec<CheckpointRegion>,
}

/// One region of guest memory to capture at the checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CheckpointRegion {
    /// Name the comparison pairs regions by.
    pub name: String,
    /// Address space the region lives in; 0 (the default) is the boot
    /// process's, a spawned child's is numbered from 1 in spawn order.
    #[serde(default)]
    pub space: u32,
    /// Guest address of the first byte, as a hex string (`"0x10000"`,
    /// optionally unprefixed) or a TOML integer.
    #[serde(deserialize_with = "de_hex_or_int")]
    pub addr: u64,
    /// Size in bytes, in the same forms as `addr`.
    #[serde(deserialize_with = "de_hex_or_int")]
    pub size: u64,
}

impl CheckpointManifest {
    /// Parse the TOML text of a manifest.
    ///
    /// # Errors
    ///
    /// Returns the TOML error, whose message names the offending
    /// field and the forms it accepts.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The regions as the observation extractor consumes them.
    pub fn region_descriptors(&self) -> Vec<RegionDescriptor> {
        self.regions
            .iter()
            .map(|r| RegionDescriptor {
                name: r.name.clone(),
                space: AddressSpaceId::new(r.space),
                addr: r.addr,
                size: r.size,
            })
            .collect()
    }
}

/// Why a manifest file could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointManifestError {
    /// Reading the file failed.
    #[error("read {path}: {source}")]
    Read {
        /// Path as given.
        path: String,
        /// The read failure.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a manifest.
    #[error("parse {path}: {source}")]
    Parse {
        /// Path as given.
        path: String,
        /// The TOML error, naming the field and the accepted forms.
        #[source]
        source: toml::de::Error,
    },
}

/// Read and parse a manifest file.
///
/// # Errors
///
/// [`CheckpointManifestError::Read`] when the file cannot be read,
/// [`CheckpointManifestError::Parse`] when it is not a manifest.
pub fn load(path: &Path) -> Result<CheckpointManifest, CheckpointManifestError> {
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| CheckpointManifestError::Read {
        path: shown.clone(),
        source,
    })?;
    CheckpointManifest::from_toml(&text).map_err(|source| CheckpointManifestError::Parse {
        path: shown,
        source,
    })
}

struct HexOrInt;

impl Visitor<'_> for HexOrInt {
    type Value = u64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a hex string like \"0x10000\" or a non-negative integer")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
        Ok(v)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
        u64::try_from(v).map_err(|_| E::invalid_value(de::Unexpected::Signed(v), &self))
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<u64, E> {
        let digits = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(digits, 16).map_err(|_| E::invalid_value(de::Unexpected::Str(s), &self))
    }
}

fn de_hex_or_int<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    d.deserialize_any(HexOrInt)
}

#[cfg(test)]
#[path = "tests/checkpoint_manifest_tests.rs"]
mod tests;
