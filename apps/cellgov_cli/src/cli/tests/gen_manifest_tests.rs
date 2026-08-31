//! Title-manifest stub generation from install records.

use super::*;
use cellgov_install::manifest::Sha256;
use cellgov_install::store::{
    Artifact, ArtifactKind, ArtifactRecord, InstallRecord, RapRecord, SourceRecord, StoreLayout,
    TitleId, TitleRecord, DEFAULT_VFS_ROOT, INSTALL_RECORD_FORMAT_VERSION,
};
use std::collections::BTreeMap;

const HDD_TITLE_ID: &str = "TEST12345";
const HDD_CONTENT_ID: &str = "XX0000-TEST12345_00-SYNTHETICHDDTITLE";
const HDD_TITLE: &str = "Synthetic HDD Title";
const DISC_TITLE_ID: &str = "TEST54321";
const DISC_TITLE: &str = "Synthetic Disc Title";

fn hdd_record() -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: format!("dev_hdd0/game/{HDD_TITLE_ID}"),
        },
        source: SourceRecord::local("pkg", Sha256([0u8; 32])),
        title: Some(TitleRecord {
            title_id: HDD_TITLE_ID.to_string(),
            content_id: HDD_CONTENT_ID.to_string(),
            category: "HG".to_string(),
            title: HDD_TITLE.to_string(),
            distribution: "psn-hdd".to_string(),
        }),
        files: BTreeMap::from([
            ("PARAM.SFO".to_string(), Sha256([1u8; 32])),
            ("USRDIR/EBOOT.BIN".to_string(), Sha256([2u8; 32])),
        ]),
        rap: Some(RapRecord {
            filename: format!("{HDD_CONTENT_ID}.rap"),
            sha256: Sha256([3u8; 32]),
        }),
    }
}

fn disc_record() -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: format!("dev_bdvd/{DISC_TITLE_ID}"),
        },
        source: SourceRecord::local("iso", Sha256([0u8; 32])),
        title: Some(TitleRecord {
            title_id: DISC_TITLE_ID.to_string(),
            content_id: DISC_TITLE_ID.to_string(),
            category: "DG".to_string(),
            title: DISC_TITLE.to_string(),
            distribution: "disc-iso".to_string(),
        }),
        files: BTreeMap::from([("PS3_GAME/USRDIR/EBOOT.BIN".to_string(), Sha256([2u8; 32]))]),
        rap: None,
    }
}

fn title_of(record: &InstallRecord) -> &TitleRecord {
    record.title.as_ref().expect("a title record has a [title]")
}

fn load_stub(record: &InstallRecord) -> crate::game::manifest::TitleManifest {
    let title = title_of(record);
    let stub = GeneratedFields::from_record(record, title).render_stub(std::path::Path::new(
        &format!("installs/titles/{}/base.install.toml", title.title_id),
    ));
    crate::game::manifest::TitleManifest::load_from_text(&stub, std::path::Path::new("stub.toml"))
        .expect("generated stub is a valid title manifest")
}

#[test]
fn hdd_stub_fills_generated_fields_with_rap() {
    let r = hdd_record();
    let g = GeneratedFields::from_record(&r, title_of(&r));
    assert_eq!(g.content_id, HDD_TITLE_ID);
    assert_eq!(g.display_name, HDD_TITLE);
    assert_eq!(g.distribution, "psn-hdd");
    assert_eq!(g.eboot_candidate, "EBOOT.BIN");
    let rap = format!("{HDD_CONTENT_ID}.rap");
    assert_eq!(g.rap_filename.as_deref(), Some(rap.as_str()));

    let manifest = load_stub(&r);
    assert_eq!(manifest.content_id, HDD_TITLE_ID);
    assert_eq!(manifest.display_name, HDD_TITLE);
    assert_eq!(manifest.rap_filename.as_deref(), Some(rap.as_str()));
    assert!(manifest.eboot_candidates.contains(&"EBOOT.BIN".to_string()));
}

#[test]
fn disc_stub_has_no_rap() {
    let r = disc_record();
    let g = GeneratedFields::from_record(&r, title_of(&r));
    assert_eq!(g.content_id, DISC_TITLE_ID);
    assert_eq!(g.distribution, "disc-iso");
    assert_eq!(g.eboot_candidate, "EBOOT.BIN");
    assert!(g.rap_filename.is_none());

    let stub = g.render_stub(std::path::Path::new("installs/stub.install.toml"));
    // No rap_filename *field* (the header comment mentions the name).
    assert!(!stub.contains("rap_filename ="));
    assert!(load_stub(&r).rap_filename.is_none());
}

#[test]
fn a_psn_hdd_record_with_no_installed_rap_names_no_rap_file() {
    let mut r = hdd_record();
    r.rap = None;
    let g = GeneratedFields::from_record(&r, title_of(&r));
    assert!(g.rap_filename.is_none());
    let stub = g.render_stub(std::path::Path::new("installs/stub.install.toml"));
    assert!(!stub.contains("rap_filename ="));
    assert!(load_stub(&r).rap_filename.is_none());
}

#[test]
fn a_record_with_no_eboot_falls_back_to_the_conventional_name() {
    let mut r = hdd_record();
    r.files = BTreeMap::new();
    let g = GeneratedFields::from_record(&r, title_of(&r));
    assert_eq!(g.eboot_candidate, "EBOOT.BIN");
}

#[test]
fn the_title_id_lookup_lands_on_the_record_cellgov_install_writes() {
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(HDD_TITLE_ID).expect("a synthetic title id is a store key"),
    };
    assert_eq!(
        base_record_under(&default_installs(), HDD_TITLE_ID),
        StoreLayout::new(DEFAULT_VFS_ROOT).record_path(&artifact),
    );
}

#[test]
fn an_installs_flag_names_the_record_directory_not_a_vfs_root() {
    let installs = std::path::Path::new("elsewhere").join("installs");
    assert_eq!(
        base_record_under(&installs, HDD_TITLE_ID),
        installs
            .join("titles")
            .join(HDD_TITLE_ID)
            .join("base.install.toml"),
    );
}

#[test]
fn stub_source_follows_the_record_distribution() {
    use crate::game::manifest::GameSource;
    for (record, is_disc) in [(disc_record(), true), (hdd_record(), false)] {
        let manifest = load_stub(&record);
        let resolved_disc = match manifest.source {
            GameSource::Disc => true,
            GameSource::Hdd => false,
            other => panic!("stub resolved to {other:?}"),
        };
        assert_eq!(
            resolved_disc,
            is_disc,
            "{} stub resolved to the wrong source",
            title_of(&record).distribution
        );
    }
}
