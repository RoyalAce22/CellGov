//! A synthetic content store built in a scratch directory, so a test
//! states what is installed rather than reading what the machine holds.
//!
//! The fixture writes the records and `firmware.toml` files as text,
//! in the schema the installers write; `cellgov_install`'s own tests read them
//! back through the store inventory and pin the two format versions
//! below against its constants.

use std::path::{Path, PathBuf};

use crate::param_sfo::build_param_sfo;
use crate::scratch::{scratch_labeled, ScratchDir};

/// The install-record schema version the synthetic records declare.
pub const INSTALL_RECORD_FORMAT_VERSION: u32 = 3;

/// The `firmware.toml` schema version a synthetic firmware tree
/// declares.
pub const FIRMWARE_MANIFEST_FORMAT_VERSION: u32 = 2;

/// A store root removed when the guard drops.
pub struct SyntheticStore {
    root: ScratchDir,
}

impl SyntheticStore {
    /// A store with nothing installed.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        let root = scratch_labeled(tag);
        std::fs::create_dir_all(root.join(".cellgov").join("installs")).unwrap();
        Self { root }
    }

    /// The VFS root the store sits under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Installs a firmware version, with its `dev_flash` tree unless
    /// `tree` is false.
    pub fn add_firmware(&self, version: &str, tree: bool) -> &Self {
        let record = format!(
            "format_version = {INSTALL_RECORD_FORMAT_VERSION}\n\n\
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
                    "format_version = {FIRMWARE_MANIFEST_FORMAT_VERSION}\n\n\
                     [firmware]\n\
                     image_version = \"{}\"\n\
                     version = \"{version}\"\n\
                     pup_sha256 = \"{}\"\n",
                    image_version(version),
                    digest('f'),
                ),
            )
            .unwrap();
        }
        self
    }

    /// Installs a title's base tree; `disc` selects which mount it
    /// backs. The tree's PARAM.SFO declares `version` under `APP_VER`,
    /// the shape a real install leaves, and no `PS3_SYSTEM_VER`.
    pub fn add_base(&self, title_id: &str, version: &str, disc: bool) -> &Self {
        self.write_base(title_id, version, disc, None, None)
    }

    /// Installs a disc title's base tree whose record names `shipped`
    /// as the firmware its disc shipped, as a disc install that
    /// registered its PUP writes it. The test adds the firmware entry
    /// itself, or leaves it out.
    pub fn add_disc_base_shipping(&self, title_id: &str, version: &str, shipped: &str) -> &Self {
        self.write_base(title_id, version, true, None, Some(shipped))
    }

    /// Installs a title's base tree whose PARAM.SFO declares
    /// `system_ver` under `PS3_SYSTEM_VER`, recorded as `[title]
    /// system_ver` the way the installer writes it.
    pub fn add_base_declaring(
        &self,
        title_id: &str,
        version: &str,
        disc: bool,
        system_ver: &str,
    ) -> &Self {
        self.write_base(title_id, version, disc, Some(system_ver), None)
    }

    fn write_base(
        &self,
        title_id: &str,
        version: &str,
        disc: bool,
        system_ver: Option<&str>,
        shipped_firmware: Option<&str>,
    ) -> &Self {
        let (distribution, store_path) = if disc {
            ("disc-iso", format!("titles/{title_id}/base/disc"))
        } else {
            ("psn-hdd", format!("titles/{title_id}/base/game"))
        };
        let declared = system_ver
            .map(|v| format!("system_ver = \"{v}\"\n"))
            .unwrap_or_default()
            + &shipped_firmware
                .map(|v| format!("shipped_firmware = \"{v}\"\n"))
                .unwrap_or_default();
        let record = format!(
            "format_version = {INSTALL_RECORD_FORMAT_VERSION}\n\n\
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
             distribution = \"{distribution}\"\n\
             {declared}",
            digest('b'),
        );
        self.write_record(&["titles", title_id], "base.install.toml", &record);
        std::fs::create_dir_all(self.root.join(&store_path)).unwrap();
        let mut entries = vec![("TITLE_ID", title_id), ("APP_VER", version)];
        if let Some(v) = system_ver {
            entries.push(("PS3_SYSTEM_VER", v));
        }
        self.write_base_param_sfo(title_id, disc, &entries);
        self
    }

    /// The PARAM.SFO of a title's base tree.
    #[must_use]
    pub fn base_param_sfo(&self, title_id: &str, disc: bool) -> PathBuf {
        let tree = self.root.join("titles").join(title_id).join("base");
        if disc {
            tree.join("disc").join("PS3_GAME").join("PARAM.SFO")
        } else {
            tree.join("game").join("PARAM.SFO")
        }
    }

    /// Replaces a base tree's PARAM.SFO with one holding `entries`.
    pub fn write_base_param_sfo(
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

    /// Installs one update version of a title.
    pub fn add_update(&self, title_id: &str, version: &str) -> &Self {
        self.write_update(title_id, version, None, None)
    }

    /// Installs one update version whose publisher metadata declares a
    /// minimum firmware (`[source] min_system_ver`), and whose own
    /// PARAM.SFO declares none.
    pub fn add_update_needing(
        &self,
        title_id: &str,
        version: &str,
        min_system_ver: Option<&str>,
    ) -> &Self {
        self.write_update(title_id, version, min_system_ver, None)
    }

    /// Installs one update version whose own PARAM.SFO declares
    /// `system_ver` under `PS3_SYSTEM_VER`, recorded as `[title]
    /// system_ver`, beside whatever the publisher metadata declares.
    pub fn add_update_declaring(
        &self,
        title_id: &str,
        version: &str,
        min_system_ver: Option<&str>,
        system_ver: &str,
    ) -> &Self {
        self.write_update(title_id, version, min_system_ver, Some(system_ver))
    }

    /// The record names the entry directory; the tree sits under its
    /// `game/` child, the shape the update installer writes.
    fn write_update(
        &self,
        title_id: &str,
        version: &str,
        min_system_ver: Option<&str>,
        system_ver: Option<&str>,
    ) -> &Self {
        let store_path = format!("titles/{title_id}/updates/{version}");
        let min = min_system_ver
            .map(|v| format!("min_system_ver = \"{v}\"\n"))
            .unwrap_or_default();
        let declared = system_ver
            .map(|v| format!("system_ver = \"{v}\"\n"))
            .unwrap_or_default();
        let record = format!(
            "format_version = {INSTALL_RECORD_FORMAT_VERSION}\n\n\
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
             distribution = \"update-pkg\"\n\
             {declared}",
            digest('c'),
        );
        self.write_record(
            &["titles", title_id],
            &format!("update-{version}.install.toml"),
            &record,
        );
        let tree = self.root.join(&store_path).join("game");
        std::fs::create_dir_all(&tree).unwrap();
        let mut entries = vec![("TITLE_ID", title_id), ("APP_VER", version)];
        if let Some(v) = system_ver {
            entries.push(("PS3_SYSTEM_VER", v));
        }
        std::fs::write(tree.join("PARAM.SFO"), build_param_sfo(&entries)).unwrap();
        self
    }

    /// The `dev_hdd0/game` tree one installed update holds.
    #[must_use]
    pub fn update_tree(&self, title_id: &str, version: &str) -> PathBuf {
        self.root
            .join("titles")
            .join(title_id)
            .join("updates")
            .join(version)
            .join("game")
    }

    /// Writes one license file into a title's own license directory.
    pub fn add_title_rap(&self, title_id: &str, filename: &str, bytes: &[u8]) -> &Self {
        let dir = self.root.join("titles").join(title_id).join("exdata");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(filename), bytes).unwrap();
        self
    }

    /// The `dev_flash` tree of one installed firmware version.
    #[must_use]
    pub fn firmware_dev_flash(&self, version: &str) -> PathBuf {
        self.firmware_entry(version).join("dev_flash")
    }

    /// The entry directory of one installed firmware version.
    #[must_use]
    pub fn firmware_entry(&self, version: &str) -> PathBuf {
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

/// A distinct 64-char digest per artifact kind, so a banner test can
/// tell which record a rendered digest came from.
fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}

/// The `image_version` a synthetic `firmware.toml` declares for one
/// firmware version. Distinct per version, so a test can tell two
/// entries apart.
#[must_use]
pub fn image_version(version: &str) -> String {
    format!("0x{}", version.replace('.', ""))
}

/// The PUP digest every synthetic firmware entry records, and the one
/// its `firmware.toml` repeats.
#[must_use]
pub fn firmware_pup_sha256() -> String {
    digest('f')
}
