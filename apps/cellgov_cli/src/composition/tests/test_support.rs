//! A synthetic store built in a scratch directory. The selection tests
//! state what is installed rather than read what this machine holds.

use std::path::{Path, PathBuf};

use cellgov_testkit::param_sfo::build_param_sfo;

/// A store root removed when the guard drops.
pub(super) struct SyntheticStore {
    root: cellgov_testkit::scratch::ScratchDir,
}

impl SyntheticStore {
    /// A store with nothing installed.
    pub(super) fn new(tag: &str) -> Self {
        let root = cellgov_testkit::scratch::scratch_labeled(tag);
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
            let dev_flash = self.firmware_dev_flash(version);
            std::fs::create_dir_all(&dev_flash).unwrap();
            std::fs::write(
                dev_flash.join("firmware.toml"),
                format!(
                    "format_version = {}\n\n\
                     [firmware]\n\
                     image_version = \"{}\"\n\
                     version = \"{version}\"\n\
                     pup_sha256 = \"{}\"\n",
                    cellgov_install::manifest::SUPPORTED_FORMAT_VERSION,
                    image_version(version),
                    digest('f'),
                ),
            )
            .unwrap();
        }
        self
    }

    /// Install a title's base tree; `disc` selects which mount it
    /// backs. The tree's PARAM.SFO declares `version` under `APP_VER`,
    /// the shape a real install leaves.
    pub(super) fn add_base(&self, title_id: &str, version: &str, disc: bool) -> &Self {
        let (distribution, store_path) = if disc {
            ("disc-iso", format!("titles/{title_id}/base/disc"))
        } else {
            ("psn-hdd", format!("titles/{title_id}/base/game"))
        };
        let record = format!(
            "format_version = 3\n\n\
             [artifact]\n\
             kind = \"title-base\"\n\
             version = \"{version}\"\n\
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
        self.write_base_param_sfo(
            title_id,
            disc,
            &[("TITLE_ID", title_id), ("APP_VER", version)],
        );
        self
    }

    /// The PARAM.SFO of a title's base tree.
    pub(super) fn base_param_sfo(&self, title_id: &str, disc: bool) -> PathBuf {
        let tree = self.root.join("titles").join(title_id).join("base");
        if disc {
            tree.join("disc").join("PS3_GAME").join("PARAM.SFO")
        } else {
            tree.join("game").join("PARAM.SFO")
        }
    }

    /// Replace a base tree's PARAM.SFO with one holding `entries`.
    pub(super) fn write_base_param_sfo(
        &self,
        title_id: &str,
        disc: bool,
        entries: &[(&str, &str)],
    ) -> &Self {
        let path = self.base_param_sfo(title_id, disc);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, build_param_sfo(entries)).unwrap();
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
        let tree = self.root.join(&store_path).join("game");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(
            tree.join("PARAM.SFO"),
            build_param_sfo(&[("TITLE_ID", title_id), ("APP_VER", version)]),
        )
        .unwrap();
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

    /// Add the single `dev_flash` mount a pre-store firmware install
    /// wrote, with its manifest inside.
    pub(super) fn add_pre_store_firmware_mount(&self) -> &Self {
        let dir = self.root.join("dev_flash");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("firmware.toml"), "format_version = 1\n").unwrap();
        self
    }

    /// Add a `<title-id>.install.toml` where the pre-store layout filed
    /// its records, directly under the records directory.
    pub(super) fn add_flat_install_record(&self, title_id: &str) -> &Self {
        self.write_record(
            &[],
            &format!("{title_id}.install.toml"),
            "format_version = 2\n",
        );
        self
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

/// A distinct 64-char digest per artifact kind, so a banner test can
/// tell which record a rendered digest came from.
fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}

/// The `image_version` the synthetic store's `firmware.toml` declares
/// for one firmware version. Distinct per version, so a test can tell
/// two entries apart.
pub(super) fn image_version(version: &str) -> String {
    format!("0x{}", version.replace('.', ""))
}

/// The PUP digest every synthetic firmware entry records, and the one
/// its `firmware.toml` repeats.
pub(super) fn firmware_pup_sha256() -> String {
    digest('f')
}
