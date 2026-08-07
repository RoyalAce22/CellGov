//! Title-manifest stub generation from install records.

use super::*;
use cellgov_install::game_install::{InstallRecord, SourceRecord, TitleRecord};
use cellgov_install::manifest::Sha256;
use std::collections::BTreeMap;

fn hdd_record() -> InstallRecord {
    InstallRecord {
        format_version: 2,
        source: SourceRecord {
            kind: "pkg".to_string(),
            sha256: Sha256([0u8; 32]),
        },
        title: TitleRecord {
            title_id: "NPUA80001".to_string(),
            content_id: "UP9000-NPUA80001_00-FLOWPS3PROMOTION".to_string(),
            category: "HG".to_string(),
            title: "flOw".to_string(),
            app_version: "01.00".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        files: BTreeMap::from([
            ("PARAM.SFO".to_string(), Sha256([1u8; 32])),
            ("USRDIR/EBOOT.BIN".to_string(), Sha256([2u8; 32])),
        ]),
        rap: None,
    }
}

fn disc_record() -> InstallRecord {
    InstallRecord {
        format_version: 2,
        source: SourceRecord {
            kind: "iso".to_string(),
            sha256: Sha256([0u8; 32]),
        },
        title: TitleRecord {
            title_id: "BCES00664".to_string(),
            content_id: "BCES00664".to_string(),
            category: "DG".to_string(),
            title: "WipEout HD".to_string(),
            app_version: "01.00".to_string(),
            distribution: "disc-iso".to_string(),
        },
        files: BTreeMap::from([("PS3_GAME/USRDIR/EBOOT.BIN".to_string(), Sha256([2u8; 32]))]),
        rap: None,
    }
}

#[test]
fn hdd_stub_fills_generated_fields_with_rap() {
    let r = hdd_record();
    let g = GeneratedFields::from_record(&r);
    assert_eq!(g.content_id, "NPUA80001");
    assert_eq!(g.display_name, "flOw");
    assert_eq!(g.distribution, "psn-hdd");
    assert_eq!(g.eboot_candidate, "EBOOT.BIN");
    assert_eq!(
        g.rap_filename.as_deref(),
        Some("UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap")
    );

    let stub = g.render_stub(std::path::Path::new("installs/NPUA80001.install.toml"));
    // The stub must parse back into a real TitleManifest.
    let manifest = crate::game::manifest::TitleManifest::load_from_text(
        &stub,
        std::path::Path::new("stub.toml"),
    )
    .expect("generated stub is a valid title manifest");
    assert_eq!(manifest.content_id, "NPUA80001");
    assert_eq!(manifest.display_name, "flOw");
    assert_eq!(
        manifest.rap_filename.as_deref(),
        Some("UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap")
    );
    assert!(manifest.eboot_candidates.contains(&"EBOOT.BIN".to_string()));
}

#[test]
fn disc_stub_has_no_rap() {
    let r = disc_record();
    let g = GeneratedFields::from_record(&r);
    assert_eq!(g.content_id, "BCES00664");
    assert_eq!(g.distribution, "disc-iso");
    assert_eq!(g.eboot_candidate, "EBOOT.BIN");
    assert!(g.rap_filename.is_none());

    let stub = g.render_stub(std::path::Path::new("installs/BCES00664.install.toml"));
    // No rap_filename *field* (the header comment mentions the name).
    assert!(!stub.contains("rap_filename ="));
    let manifest = crate::game::manifest::TitleManifest::load_from_text(
        &stub,
        std::path::Path::new("stub.toml"),
    )
    .expect("generated disc stub is a valid title manifest");
    assert!(manifest.rap_filename.is_none());
}
