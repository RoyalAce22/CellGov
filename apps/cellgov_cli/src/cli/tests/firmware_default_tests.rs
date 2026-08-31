//! Resolving the default firmware directory from the install store.

use super::*;

/// A scratch store root, removed when the guard drops.
struct StoreRoot(std::path::PathBuf);

impl StoreRoot {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("cellgov_boot_fw_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".cellgov").join("installs").join("firmware")).unwrap();
        Self(dir)
    }

    /// Write a firmware record for `version`, and the `sys/external`
    /// tree it points at unless `tree` is false.
    fn add_firmware(&self, version: &str, tree: bool) {
        let record = format!(
            "format_version = 3\n\n\
             [artifact]\n\
             kind = \"firmware\"\n\
             version = \"{version}\"\n\
             store_path = \"firmware/{version}\"\n\n\
             [source]\n\
             kind = \"pup\"\n\
             sha256 = \"{}\"\n",
            "0".repeat(64)
        );
        std::fs::write(
            self.0
                .join(".cellgov")
                .join("installs")
                .join("firmware")
                .join(format!("{version}.install.toml")),
            record,
        )
        .unwrap();
        if tree {
            std::fs::create_dir_all(self.external_dir(version)).unwrap();
        }
    }

    fn external_dir(&self, version: &str) -> std::path::PathBuf {
        self.0
            .join("firmware")
            .join(version)
            .join("dev_flash")
            .join("sys")
            .join("external")
    }
}

impl Drop for StoreRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn default_firmware_dir_follows_the_install_record_not_a_fixed_path() {
    let root = StoreRoot::new("one");
    root.add_firmware("4.93", true);
    let dir = default_firmware_dir(&root.0).unwrap();
    assert_eq!(std::path::Path::new(&dir), root.external_dir("4.93"));
}

#[test]
fn default_firmware_dir_refuses_when_no_firmware_is_installed() {
    let root = StoreRoot::new("none");
    let err = default_firmware_dir(&root.0).unwrap_err();
    assert!(err.contains("no firmware is installed"), "got: {err}");
    assert!(err.contains(DISABLE_DEFAULT_ENV), "got: {err}");
}

#[test]
fn default_firmware_dir_refuses_a_record_whose_tree_is_gone() {
    let root = StoreRoot::new("gone");
    root.add_firmware("4.93", false);
    let err = default_firmware_dir(&root.0).unwrap_err();
    assert!(err.contains("4.93 is recorded"), "got: {err}");
    assert!(err.contains("is missing"), "got: {err}");
}

#[test]
fn default_firmware_dir_refuses_to_pick_between_two_installs() {
    let root = StoreRoot::new("two");
    root.add_firmware("4.91", true);
    root.add_firmware("4.93", true);
    let err = default_firmware_dir(&root.0).unwrap_err();
    assert!(err.contains("4.91, 4.93"), "got: {err}");
    assert!(err.contains("--firmware-dir"), "got: {err}");
}
