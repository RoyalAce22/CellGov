//! A synthetic store built in a scratch directory. The selection tests
//! state what is installed rather than read what this machine holds.

use std::path::{Path, PathBuf};

/// A store root removed when the guard drops.
pub(super) struct SyntheticStore {
    root: PathBuf,
}

impl SyntheticStore {
    /// A store with nothing installed.
    pub(super) fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("cellgov_composition_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".cellgov").join("installs")).unwrap();
        Self { root }
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// Install a firmware version, with its `dev_flash` tree unless
    /// `tree` is false.
    pub(super) fn add_firmware(&self, version: &str, tree: bool) -> &Self {
        let record = format!(
            "format_version = 3\n\n\
             [artifact]\n\
             kind = \"firmware\"\n\
             version = \"{version}\"\n\
             store_path = \"firmware/{version}\"\n\n\
             [source]\n\
             kind = \"pup\"\n\
             sha256 = \"{}\"\n",
            digest('f'),
        );
        self.write_record(&["firmware"], &format!("{version}.install.toml"), &record);
        if tree {
            std::fs::create_dir_all(self.firmware_dev_flash(version)).unwrap();
        }
        self
    }

    /// Install a title's base tree; `disc` selects which mount it
    /// backs.
    pub(super) fn add_base(&self, title_id: &str, app_ver: &str, disc: bool) -> &Self {
        let (distribution, store_path) = if disc {
            ("disc-iso", format!("titles/{title_id}/base/disc"))
        } else {
            ("psn-hdd", format!("titles/{title_id}/base/game"))
        };
        let record = format!(
            "format_version = 3\n\n\
             [artifact]\n\
             kind = \"title-base\"\n\
             version = \"{app_ver}\"\n\
             store_path = \"{store_path}\"\n\n\
             [source]\n\
             kind = \"pkg\"\n\
             sha256 = \"{}\"\n\n\
             [title]\n\
             title_id = \"{title_id}\"\n\
             content_id = \"{title_id}\"\n\
             category = \"HG\"\n\
             title = \"Synthetic\"\n\
             distribution = \"{distribution}\"\n",
            digest('b'),
        );
        self.write_record(&["titles", title_id], "base.install.toml", &record);
        std::fs::create_dir_all(self.root.join(&store_path)).unwrap();
        self
    }

    /// Install one update version of a title.
    pub(super) fn add_update(&self, title_id: &str, version: &str) -> &Self {
        self.add_update_needing(title_id, version, None)
    }

    /// Install one update version that declares a minimum firmware.
    ///
    /// The record names the entry directory; the tree sits under its
    /// `game/` child, the shape `install_update` writes.
    pub(super) fn add_update_needing(
        &self,
        title_id: &str,
        version: &str,
        min_system_ver: Option<&str>,
    ) -> &Self {
        let store_path = format!("titles/{title_id}/updates/{version}");
        let min = min_system_ver
            .map(|v| format!("min_system_ver = \"{v}\"\n"))
            .unwrap_or_default();
        let record = format!(
            "format_version = 3\n\n\
             [artifact]\n\
             kind = \"title-update\"\n\
             version = \"{version}\"\n\
             store_path = \"{store_path}\"\n\n\
             [source]\n\
             kind = \"pkg\"\n\
             sha256 = \"{}\"\n\
             {min}\n\
             [title]\n\
             title_id = \"{title_id}\"\n\
             content_id = \"{title_id}\"\n\
             category = \"GD\"\n\
             title = \"Synthetic\"\n\
             distribution = \"update-pkg\"\n",
            digest('c'),
        );
        self.write_record(
            &["titles", title_id],
            &format!("update-{version}.install.toml"),
            &record,
        );
        std::fs::create_dir_all(self.root.join(&store_path).join("game")).unwrap();
        self
    }

    /// The `dev_hdd0/game` tree one installed update holds.
    pub(super) fn update_tree(&self, title_id: &str, version: &str) -> PathBuf {
        self.root
            .join("titles")
            .join(title_id)
            .join("updates")
            .join(version)
            .join("game")
    }

    /// Write one license file into a title's own license directory.
    pub(super) fn add_title_rap(&self, title_id: &str, filename: &str, bytes: &[u8]) -> &Self {
        let dir = self.root.join("titles").join(title_id).join("exdata");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(filename), bytes).unwrap();
        self
    }

    pub(super) fn firmware_dev_flash(&self, version: &str) -> PathBuf {
        self.root.join("firmware").join(version).join("dev_flash")
    }

    pub(super) fn firmware_entry(&self, version: &str) -> PathBuf {
        self.root.join("firmware").join(version)
    }

    fn write_record(&self, dirs: &[&str], name: &str, body: &str) {
        let dir = dirs
            .iter()
            .fold(self.root.join(".cellgov").join("installs"), |d, p| {
                d.join(p)
            });
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }
}

impl Drop for SyntheticStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A distinct 64-char digest per artifact kind, so a banner test can
/// tell which record a rendered digest came from.
fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}
