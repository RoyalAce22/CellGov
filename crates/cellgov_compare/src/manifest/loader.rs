//! Disk and string loaders + the `ManifestError` type.

use std::path::Path;

use super::Manifest;

/// Why manifest loading failed.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// File system error.
    #[error("manifest I/O: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse error.
    #[error("manifest parse: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Load and parse a manifest from a TOML file.
pub fn load(path: &Path) -> Result<Manifest, ManifestError> {
    let text = std::fs::read_to_string(path)?;
    parse(&text)
}

/// Parse a manifest from a TOML string.
pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    let manifest: Manifest = toml::from_str(text)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::fields::DecoderField;

    #[test]
    fn load_spu_mailbox_write_manifest() {
        let path = std::path::Path::new("../../tests/micro/spu_mailbox_write/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "spu_mailbox_write");
            assert!(m.cellgov.is_some());
            assert!(m.rpcs3.is_none());
        }
    }

    #[test]
    fn load_spu_fixed_value_manifest() {
        let path = std::path::Path::new("../../tests/micro/spu_fixed_value/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "spu_fixed_value");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 1);
            assert_eq!(m.observe.memory_regions[0].name, "result");
            assert_eq!(m.observe.memory_regions[0].size, 8);
        }
    }

    #[test]
    fn load_atomic_reservation_manifest() {
        let path = std::path::Path::new("../../tests/micro/atomic_reservation/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "atomic_reservation");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "header");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "data");
            assert_eq!(m.observe.memory_regions[1].size, 128);
        }
    }

    #[test]
    fn load_ls_to_shared_manifest() {
        let path = std::path::Path::new("../../tests/micro/ls_to_shared/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "ls_to_shared");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "header");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "data");
            assert_eq!(m.observe.memory_regions[1].size, 128);
        }
    }

    #[test]
    fn load_barrier_wakeup_manifest() {
        let path = std::path::Path::new("../../tests/micro/barrier_wakeup/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "barrier_wakeup");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "spu0_result");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "spu1_result");
            assert_eq!(m.observe.memory_regions[1].size, 8);
        }
    }
}
