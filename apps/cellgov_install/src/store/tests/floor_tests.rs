use cellgov_testkit::param_sfo::build_param_sfo;
use cellgov_testkit::store::SyntheticStore;

use super::*;
use crate::store::layout::{Artifact, TitleId, TitleTree};

const TITLE_ID: &str = "TEST12345";

fn base_record(store: &SyntheticStore) -> InstallRecord {
    let path = StoreLayout::new(store.root()).record_path(&Artifact::TitleBase {
        title_id: TitleId::new(TITLE_ID).unwrap(),
    });
    InstallRecord::parse(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn floor_of(store: &SyntheticStore, record: &InstallRecord) -> Result<String, FloorReadError> {
    base_system_ver(store.root(), record, record.title.as_ref().unwrap())
}

#[test]
fn a_disc_trees_param_sfo_sits_under_ps3_game_and_an_hdd_trees_at_its_root() {
    assert_eq!(TitleTree::Disc.param_sfo_rel(), "PS3_GAME/PARAM.SFO");
    assert_eq!(TitleTree::Game.param_sfo_rel(), "PARAM.SFO");
    let root = Path::new("store");
    assert_eq!(
        TitleTree::Disc.param_sfo_in(root),
        root.join("PS3_GAME").join("PARAM.SFO")
    );
    assert_eq!(TitleTree::Game.param_sfo_in(root), root.join("PARAM.SFO"));

    for disc in [true, false] {
        let store = SyntheticStore::new("floor-place");
        store.add_base(TITLE_ID, "01.00", disc);
        let record = base_record(&store);
        let title = record.title.as_ref().unwrap();
        assert_eq!(
            installed_param_sfo(store.root(), &record, title),
            store.base_param_sfo(TITLE_ID, disc),
            "disc = {disc}"
        );
        assert_eq!(
            title.param_sfo_rel(),
            if disc {
                "PS3_GAME/PARAM.SFO"
            } else {
                "PARAM.SFO"
            }
        );
    }
}

#[test]
fn the_floor_is_the_trees_ps3_system_ver_as_a_version_key() {
    for disc in [true, false] {
        let store = SyntheticStore::new("floor-read");
        store.add_base_declaring(TITLE_ID, "01.00", disc, "01.5000");
        assert_eq!(floor_of(&store, &base_record(&store)).unwrap(), "1.50");
    }
}

#[test]
fn a_table_declaring_no_system_ver_is_refused_by_name() {
    let store = SyntheticStore::new("floor-none");
    store.add_base(TITLE_ID, "01.00", false);
    let err = floor_of(&store, &base_record(&store)).unwrap_err();
    assert!(
        matches!(&err, FloorReadError::NoSystemVer { sfo } if *sfo == store.base_param_sfo(TITLE_ID, false)),
        "{err:?}"
    );
}

#[test]
fn a_missing_table_is_a_read_refusal() {
    let store = SyntheticStore::new("floor-gone");
    store.add_base_declaring(TITLE_ID, "01.00", true, "01.5000");
    std::fs::remove_file(store.base_param_sfo(TITLE_ID, true)).unwrap();
    let err = floor_of(&store, &base_record(&store)).unwrap_err();
    assert!(matches!(err, FloorReadError::Read { .. }), "{err:?}");
}

#[test]
fn bytes_that_are_no_param_sfo_are_a_parse_refusal() {
    let store = SyntheticStore::new("floor-junk");
    store.add_base_declaring(TITLE_ID, "01.00", false, "01.5000");
    std::fs::write(store.base_param_sfo(TITLE_ID, false), b"not a table").unwrap();
    let err = floor_of(&store, &base_record(&store)).unwrap_err();
    assert!(matches!(err, FloorReadError::Parse { .. }), "{err:?}");
}

#[test]
fn a_system_ver_of_another_shape_is_refused() {
    let store = SyntheticStore::new("floor-shape");
    store.add_base(TITLE_ID, "01.00", false);
    store.write_base_param_sfo(TITLE_ID, false, &[("PS3_SYSTEM_VER", "1.5")]);
    let err = floor_of(&store, &base_record(&store)).unwrap_err();
    assert!(
        matches!(&err, FloorReadError::Shape { source, .. } if source.value == "1.5"),
        "{err:?}"
    );
}

#[test]
fn a_table_is_held_to_the_digest_the_record_holds_for_it() {
    let store = SyntheticStore::new("floor-digest");
    store.add_base_declaring(TITLE_ID, "01.00", true, "01.5000");
    let bytes = build_param_sfo(&[
        ("TITLE_ID", TITLE_ID),
        ("APP_VER", "01.00"),
        ("PS3_SYSTEM_VER", "01.5000"),
    ]);
    assert_eq!(
        std::fs::read(store.base_param_sfo(TITLE_ID, true)).unwrap(),
        bytes,
        "the fixture's table is the one digested below"
    );
    let mut record = base_record(&store);

    record
        .files
        .insert("PS3_GAME/PARAM.SFO".to_string(), Sha256(sha256_of(&bytes)));
    assert_eq!(floor_of(&store, &record).unwrap(), "1.50");

    let other = Sha256([9u8; 32]);
    record.files.insert("PS3_GAME/PARAM.SFO".to_string(), other);
    let err = floor_of(&store, &record).unwrap_err();
    let FloorReadError::DigestMismatch {
        found, recorded, ..
    } = err
    else {
        panic!("expected a digest mismatch, got {err:?}");
    };
    assert_eq!(found, Sha256(sha256_of(&bytes)));
    assert_eq!(recorded, other);
}

#[test]
fn a_digest_under_another_key_is_not_consulted() {
    let store = SyntheticStore::new("floor-key");
    store.add_base_declaring(TITLE_ID, "01.00", true, "01.5000");
    let mut record = base_record(&store);
    record
        .files
        .insert("PARAM.SFO".to_string(), Sha256([9u8; 32]));
    assert_eq!(
        floor_of(&store, &record).unwrap(),
        "1.50",
        "a disc tree's table is keyed under PS3_GAME/"
    );
}
