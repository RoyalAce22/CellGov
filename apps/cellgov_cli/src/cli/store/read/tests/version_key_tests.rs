use super::*;

use std::collections::BTreeMap;

use cellgov_install::store::TitleTree;
use cellgov_ps3_abi::title_tree::DISC_GAME_DIR;
use cellgov_testkit::param_sfo::build_param_sfo;
use cellgov_testkit::scratch::{scratch_labeled, ScratchDir};

use crate::composition::inventory::{BaseEntry, UpdateEntry};
use crate::game::manifest::TitleRegistry;

/// Placeholder identity: these cases write every tree by hand and name
/// no installed corpus.
const TITLE_ID: &str = "TEST00000";

struct Fixture {
    root: ScratchDir,
}

impl Fixture {
    fn new(label: &str) -> Self {
        Self {
            root: scratch_labeled(label),
        }
    }

    fn view(&self) -> StoreView {
        StoreView {
            root: self.root.to_path_buf(),
            inventory: StoreInventory::read(Path::new("no-such-store-root"))
                .expect("an absent root reads as an empty store"),
            registry: TitleRegistry::default(),
            fixtures: PathBuf::from("no-such-fixtures"),
        }
    }

    fn base_dir(&self) -> PathBuf {
        self.root.join("dev_hdd0").join("game").join(TITLE_ID)
    }

    fn update_dir(&self, version: &str) -> PathBuf {
        self.root
            .join("titles")
            .join(TITLE_ID)
            .join("updates")
            .join(version)
            .join("game")
    }

    fn write_sfo(&self, dir: &Path, entries: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).expect("create the tree");
        std::fs::write(dir.join("PARAM.SFO"), build_param_sfo(entries)).expect("write PARAM.SFO");
    }

    fn base(&self, recorded: &str) -> BaseEntry {
        BaseEntry {
            version: recorded.to_string(),
            dir: self.base_dir(),
            tree: TitleTree::Game,
            distribution: "psn-hdd".to_string(),
            source_sha256: "ab".repeat(32),
            system_ver: None,
            shipped_firmware: None,
        }
    }

    fn entry(&self, base: BaseEntry, updates: &[&str]) -> TitleEntry {
        TitleEntry {
            title_id: TITLE_ID.to_string(),
            base: Some(base),
            updates: updates
                .iter()
                .map(|version| {
                    (
                        (*version).to_string(),
                        UpdateEntry {
                            version: (*version).to_string(),
                            dir: self.update_dir(version),
                            source_sha256: "cd".repeat(32),
                            min_system_ver: None,
                            system_ver: None,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            exdata_dir: self.root.join("titles").join(TITLE_ID).join("exdata"),
        }
    }
}

fn base_of(doc: &TitleDoc) -> &BaseDoc {
    doc.base.as_ref().expect("the entry holds a base")
}

#[test]
fn a_base_whose_table_names_app_ver_carries_that_key() {
    let fx = Fixture::new("vk_app_ver");
    fx.write_sfo(
        &fx.base_dir(),
        &[("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")],
    );
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version, "01.00");
    assert_eq!(base.version_key.as_deref(), Some("app_ver"));
    assert_eq!(base.param_sfo_error, None);
}

#[test]
fn a_base_whose_table_names_only_version_carries_the_sfo_version_key() {
    let fx = Fixture::new("vk_sfo_version");
    fx.write_sfo(
        &fx.base_dir(),
        &[("TITLE_ID", TITLE_ID), ("VERSION", "01.00")],
    );
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version_key.as_deref(), Some("sfo_version"));
    assert_eq!(base.param_sfo_error, None);
}

#[test]
fn a_table_naming_another_version_than_the_record_leaves_the_key_absent_and_names_both() {
    let fx = Fixture::new("vk_mismatch");
    fx.write_sfo(
        &fx.base_dir(),
        &[("TITLE_ID", TITLE_ID), ("APP_VER", "01.05")],
    );
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version, "01.00", "the record's version stands");
    assert_eq!(base.version_key, None);
    let why = base
        .param_sfo_error
        .as_deref()
        .expect("the table did not confirm the record");
    assert!(
        why.contains("app_ver 01.05") && why.contains("\"01.00\""),
        "{why}"
    );
}

#[test]
fn a_missing_table_leaves_the_key_absent_and_names_the_file() {
    let fx = Fixture::new("vk_missing");
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version_key, None);
    let why = base.param_sfo_error.as_deref().expect("no table was read");
    let table = fx.base_dir().join("PARAM.SFO").display().to_string();
    assert!(
        why.starts_with("read ") && why.contains(&table),
        "the error names the file it could not read: {why}"
    );
}

#[test]
fn a_table_that_does_not_parse_leaves_the_key_absent_and_names_the_file() {
    let fx = Fixture::new("vk_unparsable");
    std::fs::create_dir_all(fx.base_dir()).expect("create the tree");
    std::fs::write(fx.base_dir().join("PARAM.SFO"), b"not an sfo").expect("write the stub");
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version_key, None);
    let why = base
        .param_sfo_error
        .as_deref()
        .expect("the stub does not parse");
    let table = fx.base_dir().join("PARAM.SFO").display().to_string();
    assert!(
        why.starts_with(&table) && why.contains("too small for header"),
        "the error names the file and the parse failure: {why}"
    );
}

#[test]
fn a_table_naming_no_version_over_a_record_that_does_leaves_the_key_absent_and_says_so() {
    let fx = Fixture::new("vk_none_over_recorded");
    fx.write_sfo(&fx.base_dir(), &[("TITLE_ID", TITLE_ID)]);
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version, "01.00", "the record's version stands");
    assert_eq!(base.version_key, None);
    let why = base
        .param_sfo_error
        .as_deref()
        .expect("a table naming no version does not confirm a recorded one");
    assert!(
        why.contains("no version key") && why.contains("\"01.00\""),
        "{why}"
    );
}

#[test]
fn a_disc_base_reads_the_table_under_its_game_directory() {
    let fx = Fixture::new("vk_disc");
    let disc_dir = fx.root.join("titles").join(TITLE_ID).join("disc");
    // The table at the disc root is a decoy: the disc layout keeps the
    // title's table under its game directory, and that is the one the
    // record was written from.
    fx.write_sfo(&disc_dir, &[("TITLE_ID", TITLE_ID), ("APP_VER", "09.99")]);
    fx.write_sfo(
        &disc_dir.join(DISC_GAME_DIR),
        &[("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")],
    );
    let base = BaseEntry {
        version: "01.00".to_string(),
        dir: disc_dir,
        tree: TitleTree::Disc,
        distribution: "disc".to_string(),
        source_sha256: "ab".repeat(32),
        system_ver: None,
        shipped_firmware: None,
    };
    let doc = fx.view().title_doc(&fx.entry(base, &[]));
    let base = base_of(&doc);
    assert_eq!(base.tree, "disc");
    assert_eq!(base.version_key.as_deref(), Some("app_ver"));
    assert_eq!(base.param_sfo_error, None);
}

#[test]
fn a_table_naming_no_version_over_an_empty_record_carries_neither_field() {
    let fx = Fixture::new("vk_none");
    fx.write_sfo(&fx.base_dir(), &[("TITLE_ID", TITLE_ID)]);
    let doc = fx.view().title_doc(&fx.entry(fx.base(""), &[]));
    let base = base_of(&doc);
    assert_eq!(base.version, "");
    assert_eq!(base.version_key, None);
    assert_eq!(base.param_sfo_error, None);
}

#[test]
fn an_update_names_the_key_of_its_own_table() {
    let fx = Fixture::new("vk_update");
    fx.write_sfo(
        &fx.base_dir(),
        &[("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")],
    );
    fx.write_sfo(
        &fx.update_dir("02.51"),
        &[("TITLE_ID", TITLE_ID), ("VERSION", "02.51")],
    );
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &["02.51"]));
    assert_eq!(base_of(&doc).version_key.as_deref(), Some("app_ver"));
    assert_eq!(doc.updates[0].version_key.as_deref(), Some("sfo_version"));
    assert_eq!(doc.updates[0].param_sfo_error, None);
}

#[test]
fn the_key_serializes_under_version_key_and_an_absent_one_is_omitted() {
    let fx = Fixture::new("vk_json");
    fx.write_sfo(
        &fx.base_dir(),
        &[("TITLE_ID", TITLE_ID), ("APP_VER", "01.00")],
    );
    let doc = fx.view().title_doc(&fx.entry(fx.base("01.00"), &["02.51"]));
    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&doc).expect("serialize")).expect("parse");
    assert_eq!(json["base"]["version"], "01.00");
    assert_eq!(json["base"]["version_key"], "app_ver");
    assert!(
        json["base"].get("param_sfo_error").is_none(),
        "a confirmed table names no error: {json}"
    );
    let update = &json["updates"][0];
    assert!(
        update.get("version_key").is_none(),
        "an update with no table names no key: {update}"
    );
    assert!(
        update["param_sfo_error"].as_str().is_some(),
        "an update with no table names why: {update}"
    );
}
