//! The read of the runner's installed firmware version.

use super::*;

struct Install(cellgov_testkit::scratch::ScratchDir);

impl Install {
    fn new(name: &str) -> Self {
        Self(cellgov_testkit::scratch::scratch_labeled(name))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Write `version.txt` into a `dev_flash` tree at `rel`.
    fn with_firmware(&self, rel: &str, text: &str) -> &Self {
        let mut path = self.path().join(rel);
        for c in VERSION_TXT_COMPONENTS {
            path.push(c);
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        self
    }

    fn with_vfs_config(&self, text: &str) -> &Self {
        self.with_vfs_config_under("", text)
    }

    /// Write the mapping file under the settings root at `rel`.
    fn with_vfs_config_under(&self, rel: &str, text: &str) -> &Self {
        let dir = self.path().join(rel).join(CONFIG_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(VFS_CONFIG_FILE), text).unwrap();
        self
    }
}

const V493: &str = "release:04.9300:\nbuild:68500,20260108:host\n";
const V492: &str = "release:04.9200:\nbuild:68466,20250218:host\n";

#[test]
fn an_unconfigured_install_reads_its_default_dev_flash() {
    let install = Install::new("runner-default");
    install.with_firmware("dev_flash", V493);
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_mapped_dev_flash_wins_over_the_default() {
    let install = Install::new("runner-mapped");
    install.with_firmware("dev_flash", V492);
    install.with_firmware("elsewhere", V493);
    let mapped = install.path().join("elsewhere").display().to_string();
    install.with_vfs_config(&format!("/dev_flash/: \"{}\"\n", mapped.replace('\\', "/")));
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_mapping_relative_to_the_installation_expands() {
    let install = Install::new("runner-emudir");
    install.with_firmware("other_flash", V493);
    install.with_vfs_config("/dev_flash/: $(EmulatorDir)other_flash/\n");
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_relocated_emulator_dir_moves_the_default_with_it() {
    let install = Install::new("runner-relocated");
    let elsewhere = install.path().join("moved");
    install.with_firmware("moved/dev_flash", V493);
    install.with_vfs_config(&format!(
        "$(EmulatorDir): {}\n",
        elsewhere.display().to_string().replace('\\', "/")
    ));
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn an_empty_mapping_leaves_the_default_in_place() {
    let install = Install::new("runner-empty-mapping");
    install.with_firmware("dev_flash", V493);
    install.with_vfs_config("/dev_flash/: \"\"\n$(EmulatorDir): \"\"\n");
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_neighbouring_key_is_not_read_as_the_dev_flash_one() {
    let install = Install::new("runner-neighbour");
    install.with_firmware("dev_flash", V493);
    install.with_firmware("second", V492);
    let second = install
        .path()
        .join("second")
        .display()
        .to_string()
        .replace('\\', "/");
    install.with_vfs_config(&format!("/dev_flash2/: \"{second}\"\n"));
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn an_install_with_no_firmware_names_the_tree_it_looked_in() {
    let install = Install::new("runner-no-firmware");
    let err = firmware_version(install.path()).unwrap_err();
    let rendered = err.to_string();
    assert!(
        matches!(err, RunnerFirmwareError::VersionUnreadable { .. }),
        "{rendered}"
    );
    assert!(rendered.contains("dev_flash"), "{rendered}");
}

#[test]
fn a_version_file_with_no_release_record_is_unparseable() {
    let install = Install::new("runner-no-release");
    install.with_firmware("dev_flash", "build:68500,20260108:host\n");
    assert!(matches!(
        firmware_version(install.path()).unwrap_err(),
        RunnerFirmwareError::VersionUnparseable { .. }
    ));
}

#[test]
fn an_unreadable_mapping_file_is_refused_rather_than_read_as_absent() {
    let install = Install::new("runner-vfs-unreadable");
    install.with_firmware("dev_flash", V493);
    // A directory in the mapping file's place fails the read with
    // something other than NotFound on every host.
    std::fs::create_dir_all(install.path().join(CONFIG_DIR).join(VFS_CONFIG_FILE)).unwrap();
    assert!(matches!(
        firmware_version(install.path()).unwrap_err(),
        RunnerFirmwareError::VfsUnreadable { .. }
    ));
}

/// The whole block the runner writes, defaults included. The nested USB
/// device map's indented keys must not read as top-level mappings.
#[test]
fn the_mapping_block_the_runner_writes_reads_the_tree_it_names() {
    let install = Install::new("runner-full-block");
    install.with_firmware("dev_flash", V492);
    install.with_firmware("flash", V493);
    install.with_vfs_config(concat!(
        "$(EmulatorDir): \"\"\n",
        "/dev_hdd0/: $(EmulatorDir)dev_hdd0/\n",
        "/dev_hdd1/: $(EmulatorDir)dev_hdd1/\n",
        "/dev_flash/: $(EmulatorDir)flash/\n",
        "/dev_flash2/: $(EmulatorDir)dev_flash2/\n",
        "/dev_flash3/: $(EmulatorDir)dev_flash3/\n",
        "/dev_bdvd/: \"\"\n",
        "/games/: $(EmulatorDir)games/\n",
        "/app_home/: \"\"\n",
        "/dev_usb***/:\n",
        "  /dev_usb000:\n",
        "    Path: $(EmulatorDir)dev_usb000/\n",
        "    Serial: \"\"\n",
    ));
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_commented_out_mapping_is_not_read_as_one() {
    let install = Install::new("runner-comment");
    install.with_firmware("dev_flash", V493);
    install.with_firmware("old", V492);
    install.with_vfs_config("# /dev_flash/: $(EmulatorDir)old/\n");
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_portable_settings_root_displaces_the_installation_root() {
    let install = Install::new("runner-portable");
    install.with_firmware("dev_flash", V492);
    install.with_firmware("portable/dev_flash", V493);
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}

#[test]
fn a_portable_install_reads_the_mapping_beside_its_own_settings() {
    let install = Install::new("runner-portable-mapped");
    install.with_firmware("portable/dev_flash", V492);
    install.with_firmware("portable/flash", V493);
    // The installation root's mapping file belongs to a non-portable
    // run and must not be the one read.
    install.with_vfs_config("/dev_flash/: $(EmulatorDir)dev_flash/\n");
    install.with_vfs_config_under(PORTABLE_DIR, "/dev_flash/: $(EmulatorDir)flash/\n");
    assert_eq!(firmware_version(install.path()).unwrap(), "4.93");
}
