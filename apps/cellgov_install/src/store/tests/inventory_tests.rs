use super::*;

use cellgov_testkit::scratch::{scratch_labeled, ScratchDir};

use crate::game_install::sha256_of;
use crate::store::layout::{Artifact, VersionKey};
use crate::store::record::{
    ArtifactRecord, SourceRecord, TitleRecord, DISC_DISTRIBUTION, INSTALL_RECORD_FORMAT_VERSION,
    PSN_HDD_DISTRIBUTION,
};

const SYNTHETIC_TITLE_ID: &str = "TEST00000";

/// A store in a scratch directory, written through the store's own
/// layout and record serializer.
struct Store {
    root: ScratchDir,
    layout: StoreLayout,
}

impl Store {
    fn new(tag: &str) -> Self {
        let root = scratch_labeled(tag);
        let layout = StoreLayout::new(root.to_path_buf());
        std::fs::create_dir_all(layout.installs_dir()).unwrap();
        Self { root, layout }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn firmware(version: &str) -> Artifact {
        Artifact::Firmware {
            version: VersionKey::new(version).unwrap(),
        }
    }

    fn base(title_id: &str) -> Artifact {
        Artifact::TitleBase {
            title_id: TitleId::new(title_id).unwrap(),
        }
    }

    fn update(title_id: &str, version: &str) -> Artifact {
        Artifact::TitleUpdate {
            title_id: TitleId::new(title_id).unwrap(),
            version: VersionKey::new(version).unwrap(),
        }
    }

    fn record_path(&self, artifact: &Artifact) -> PathBuf {
        self.layout.record_path(artifact)
    }

    fn store_path(&self, dir: &Path) -> String {
        self.layout.store_path_of(dir).unwrap()
    }

    fn write(&self, artifact: &Artifact, record: &InstallRecord) {
        let path = self.record_path(artifact);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, record.to_toml().unwrap()).unwrap();
    }

    fn add_firmware(&self, version: &str) -> &Self {
        let artifact = Self::firmware(version);
        self.write(
            &artifact,
            &InstallRecord {
                format_version: INSTALL_RECORD_FORMAT_VERSION,
                artifact: ArtifactRecord {
                    kind: ArtifactKind::Firmware,
                    version: version.to_string(),
                    store_path: self.store_path(&self.firmware_entry(version)),
                },
                source: SourceRecord::local("pup", sha256_of(b"pup")),
                title: None,
                files: BTreeMap::new(),
                rap: None,
                core_os: None,
            },
        );
        self
    }

    fn title_block(title_id: &str, category: &str, distribution: &str) -> TitleRecord {
        TitleRecord {
            title_id: title_id.to_string(),
            content_id: title_id.to_string(),
            category: category.to_string(),
            title: "Synthetic".to_string(),
            distribution: distribution.to_string(),
            system_ver: None,
            shipped_firmware: None,
        }
    }

    /// Installs a title's base; `disc` selects which mount it backs.
    fn add_base(&self, title_id: &str, version: &str, disc: bool) -> &Self {
        let artifact = Self::base(title_id);
        let (tree, distribution) = if disc {
            (TitleTree::Disc, DISC_DISTRIBUTION)
        } else {
            (TitleTree::Game, PSN_HDD_DISTRIBUTION)
        };
        let dir = self.layout.entry_dir(&artifact).join(tree.dir_name());
        self.write(
            &artifact,
            &InstallRecord {
                format_version: INSTALL_RECORD_FORMAT_VERSION,
                artifact: ArtifactRecord {
                    kind: ArtifactKind::TitleBase,
                    version: version.to_string(),
                    store_path: self.store_path(&dir),
                },
                source: SourceRecord::local("pkg", sha256_of(b"base")),
                title: Some(Self::title_block(title_id, "HG", distribution)),
                files: BTreeMap::new(),
                rap: None,
                core_os: None,
            },
        );
        self
    }

    /// Installs one update version of a title.
    fn add_update(&self, title_id: &str, version: &str) -> &Self {
        let artifact = Self::update(title_id, version);
        self.write(
            &artifact,
            &InstallRecord {
                format_version: INSTALL_RECORD_FORMAT_VERSION,
                artifact: ArtifactRecord {
                    kind: ArtifactKind::TitleUpdate,
                    version: version.to_string(),
                    store_path: self.store_path(&self.layout.entry_dir(&artifact)),
                },
                source: SourceRecord::local("pkg", sha256_of(b"update")),
                title: Some(Self::title_block(title_id, "GD", "update-pkg")),
                files: BTreeMap::new(),
                rap: None,
                core_os: None,
            },
        );
        self
    }

    /// Writes one license file into a title's own license directory.
    fn add_title_rap(&self, title_id: &str, filename: &str) -> &Self {
        let dir = self
            .layout
            .title_exdata_dir(&TitleId::new(title_id).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(filename), b"0123456789abcdef").unwrap();
        self
    }

    fn firmware_entry(&self, version: &str) -> PathBuf {
        self.layout.entry_dir(&Self::firmware(version))
    }

    fn update_tree(&self, title_id: &str, version: &str) -> PathBuf {
        self.layout
            .entry_dir(&Self::update(title_id, version))
            .join(TitleTree::Game.dir_name())
    }

    /// Adds the single `dev_flash` mount a pre-store firmware install
    /// wrote, with its manifest inside.
    fn add_pre_store_firmware_mount(&self) -> &Self {
        let dir = self.root.join("dev_flash");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("firmware.toml"), "format_version = 1\n").unwrap();
        self
    }

    /// Adds a `<title-id>.install.toml` where the pre-store layout filed
    /// its records, directly under the records directory.
    fn add_flat_install_record(&self, title_id: &str) -> &Self {
        std::fs::write(
            self.layout
                .installs_dir()
                .join(format!("{title_id}{INSTALL_RECORD_SUFFIX}")),
            "format_version = 2\n",
        )
        .unwrap();
        self
    }
}

#[test]
fn empty_store_reads_as_nothing_installed() {
    let store = Store::new("inv_empty");
    let inventory = StoreInventory::read(store.root()).unwrap();
    assert!(inventory.firmware_versions().is_empty());
    assert!(inventory.title("NPAA00001").is_none());
}

#[test]
fn firmware_entry_dir_follows_the_record_store_path() {
    let store = Store::new("inv_fw");
    store.add_firmware("4.93");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.firmware("4.93").unwrap();
    assert_eq!(entry.entry_dir, store.firmware_entry("4.93"));
    assert_eq!(
        entry.dev_flash_dir(),
        store.firmware_entry("4.93").join("dev_flash")
    );
}

/// A moved tree is where its record says, not where the layout would
/// have put it.
#[test]
fn a_firmware_entry_dir_is_the_recorded_path_not_the_layout_default() {
    let store = Store::new("inv_fw_moved");
    store.add_firmware("4.93");
    let path = store.record_path(&Store::firmware("4.93"));
    let mut record = InstallRecord::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    record.artifact.store_path = "relocated/4.93".to_string();
    std::fs::write(&path, record.to_toml().unwrap()).unwrap();
    let inventory = StoreInventory::read(store.root()).unwrap();
    assert_eq!(
        inventory.firmware("4.93").unwrap().entry_dir,
        store.root().join("relocated").join("4.93")
    );
}

#[test]
fn disc_distribution_selects_the_disc_tree() {
    let store = Store::new("inv_disc");
    store.add_base("BLAA00001", "02.00", true);
    store.add_base("NPAA00001", "01.00", false);
    let inventory = StoreInventory::read(store.root()).unwrap();
    let base = inventory.title("BLAA00001").unwrap().base.as_ref().unwrap();
    assert_eq!(base.tree, TitleTree::Disc);
    assert_eq!(base.version, "02.00");
    let hdd = inventory.title("NPAA00001").unwrap().base.as_ref().unwrap();
    assert_eq!(hdd.tree, TitleTree::Game);
}

#[test]
fn updates_are_ordered_by_version_string() {
    let store = Store::new("inv_updates");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    store.add_update("NPAA00001", "01.01");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.title("NPAA00001").unwrap();
    assert_eq!(
        entry.updates.keys().collect::<Vec<_>>(),
        vec!["01.01", "02.51"]
    );
    assert_eq!(entry.candidates(), vec!["base", "01.01", "02.51"]);
}

#[test]
fn an_update_without_a_base_reads_as_an_orphan() {
    let store = Store::new("inv_orphan");
    store.add_update("NPAA00001", "01.01");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let entry = inventory.title("NPAA00001").unwrap();
    assert!(entry.base.is_none());
    assert_eq!(entry.candidates(), vec!["01.01"]);
}

#[test]
fn a_record_this_build_does_not_read_is_named_rather_than_skipped() {
    let store = Store::new("inv_v2");
    let path = store.record_path(&Store::firmware("4.93"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        "format_version = 2\n\n[artifact]\nkind = \"firmware\"\nversion = \"4.93\"\n\
         store_path = \"firmware/4.93\"\n\n[source]\nkind = \"pup\"\nsha256 = \"00\"\n",
    )
    .unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::ParseRecord { .. }),
        "got: {err}"
    );
}

#[test]
fn a_record_filed_under_the_wrong_kind_is_refused() {
    let store = Store::new("inv_kind");
    store.add_base("NPAA00001", "01.00", false);
    // A base record filed where an update record belongs.
    let base = std::fs::read_to_string(store.record_path(&Store::base("NPAA00001"))).unwrap();
    std::fs::write(
        store.record_path(&Store::update("NPAA00001", "01.01")),
        base,
    )
    .unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::KindMismatch { .. }),
        "got: {err}"
    );
}

#[test]
fn a_record_naming_another_title_is_refused() {
    let store = Store::new("inv_titleid");
    store.add_base("NPAA00001", "01.00", false);
    let record = std::fs::read_to_string(store.record_path(&Store::base("NPAA00001"))).unwrap();
    let foreign = store.record_path(&Store::base("BLAA00001"));
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(foreign, record).unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::TitleIdMismatch { .. }),
        "got: {err}"
    );
}

#[test]
fn exdata_roots_list_the_live_directory_before_the_per_title_ones() {
    let store = Store::new("inv_exdata");
    store.add_base("NPAA00001", "01.00", false);
    store.add_title_rap("NPAA00001", "a.rap");
    let live = store.root().join("dev_hdd0/home/00000001/exdata");
    std::fs::create_dir_all(&live).unwrap();
    let inventory = StoreInventory::read(store.root()).unwrap();
    assert_eq!(
        inventory.exdata_roots().unwrap(),
        vec![live, store.root().join("titles/NPAA00001/exdata")]
    );
}

#[test]
fn an_update_entry_names_the_game_tree_not_the_entry_directory() {
    let store = Store::new("inv_update_tree");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let inventory = StoreInventory::read(store.root()).unwrap();
    let update = &inventory.title("NPAA00001").unwrap().updates["02.51"];
    assert_eq!(update.dir, store.update_tree("NPAA00001", "02.51"));
}

#[test]
fn a_second_record_claiming_one_version_is_refused_rather_than_replacing_the_first() {
    let store = Store::new("inv_dup");
    store.add_firmware("4.93");
    let path = store.record_path(&Store::firmware("4.93"));
    let record = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path.with_file_name("4.93-copy.install.toml"), record).unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::MisfiledRecord { .. }),
        "got: {err}"
    );
}

#[test]
fn an_update_record_filed_under_another_version_is_refused() {
    let store = Store::new("inv_misfiled_update");
    store.add_base("NPAA00001", "01.00", false);
    store.add_update("NPAA00001", "02.51");
    let filed = store.record_path(&Store::update("NPAA00001", "02.51"));
    let record = std::fs::read_to_string(&filed).unwrap();
    std::fs::remove_file(&filed).unwrap();
    std::fs::write(
        store.record_path(&Store::update("NPAA00001", "02.50")),
        record,
    )
    .unwrap();
    let err = StoreInventory::read(store.root()).unwrap_err();
    assert!(
        matches!(err, InventoryError::MisfiledRecord { .. }),
        "got: {err}"
    );
}

/// The listings the uninstallers read name the versions the records are
/// filed as, in byte order, and skip every other file.
#[test]
fn the_record_listings_name_the_filed_versions_in_byte_order() {
    let store = Store::new("inv_listings");
    store
        .add_firmware("4.93")
        .add_firmware("1.0-x")
        .add_firmware("1.0");
    store.add_base(SYNTHETIC_TITLE_ID, "01.00", false);
    store
        .add_update(SYNTHETIC_TITLE_ID, "1.9")
        .add_update(SYNTHETIC_TITLE_ID, "1.9-x")
        .add_update(SYNTHETIC_TITLE_ID, "1.10");
    std::fs::write(
        store.layout.firmware_records_dir().join("notes.txt"),
        "not a record",
    )
    .unwrap();
    assert_eq!(
        firmware_record_versions(&store.layout).unwrap(),
        vec!["1.0", "1.0-x", "4.93"]
    );
    assert_eq!(
        update_record_versions(&store.layout, &TitleId::new(SYNTHETIC_TITLE_ID).unwrap()).unwrap(),
        vec!["1.10", "1.9", "1.9-x"]
    );
    assert!(
        update_record_versions(&store.layout, &TitleId::new("NPAA00001").unwrap())
            .unwrap()
            .is_empty()
    );
}

// The pre-store refusal at [`StoreInventory::read`], the entry point
// every command that reads the store shares.

fn refusal(store: &Store) -> String {
    match StoreInventory::read(store.root()) {
        Err(InventoryError::PreStore(e)) => e.to_string(),
        other => panic!("expected a pre-store refusal, got {other:?}"),
    }
}

#[test]
fn the_pre_store_firmware_mount_refuses_the_read_and_names_the_reinstall() {
    let store = Store::new("prestore_fw");
    store.add_pre_store_firmware_mount();
    let msg = refusal(&store);
    assert!(msg.contains("dev_flash"), "{msg}");
    assert!(msg.contains("cellgov firmware install"), "{msg}");
}

/// A flat record is invisible to the kind-scoped walk.
#[test]
fn a_flat_install_record_refuses_the_read_rather_than_reading_as_empty() {
    let store = Store::new("prestore_record");
    store.add_flat_install_record(SYNTHETIC_TITLE_ID);
    let msg = refusal(&store);
    assert!(msg.contains(SYNTHETIC_TITLE_ID), "{msg}");
    assert!(msg.contains("cellgov title install"), "{msg}");
}

#[test]
fn a_store_holding_a_firmware_and_a_title_reads_without_a_refusal() {
    let store = Store::new("prestore_clean");
    store.add_firmware("4.93");
    store.add_base(SYNTHETIC_TITLE_ID, "01.00", false);
    let inventory = StoreInventory::read(store.root()).expect("the store layout is not refused");
    assert_eq!(inventory.firmware_versions(), vec!["4.93".to_string()]);
    assert!(inventory.title(SYNTHETIC_TITLE_ID).is_some());
}
