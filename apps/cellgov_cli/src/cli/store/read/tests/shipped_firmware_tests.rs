//! The shipped firmware of a disc base, from the record to the report.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::store::TitleTree;

use super::*;
use crate::cli::store::read::collect::StoreView;
use cellgov_boot::manifest::TitleRegistry;
use cellgov_install::store::inventory::{BaseEntry, StoreInventory, TitleEntry};

const TITLE_ID: &str = "TEST00000";

fn disc_entry(shipped_firmware: Option<&str>) -> TitleEntry {
    TitleEntry {
        title_id: TITLE_ID.to_string(),
        base: Some(BaseEntry {
            version: "02.00".to_string(),
            dir: PathBuf::from("vfs").join("dev_bdvd").join(TITLE_ID),
            tree: TitleTree::Disc,
            distribution: "disc-iso".to_string(),
            source_sha256: "ab".repeat(32),
            system_ver: Some("02.7600".to_string()),
            shipped_firmware: shipped_firmware.map(str::to_string),
        }),
        updates: BTreeMap::new(),
        exdata_dir: PathBuf::from("vfs")
            .join("titles")
            .join(TITLE_ID)
            .join("exdata"),
    }
}

fn view() -> StoreView {
    StoreView {
        root: PathBuf::from("vfs"),
        inventory: StoreInventory::read(Path::new("no-such-store-root"))
            .expect("an absent root reads as an empty store"),
        registry: TitleRegistry::default(),
        fixtures: PathBuf::from("no-such-fixtures"),
    }
}

#[test]
fn the_document_carries_the_shipped_firmware_the_record_holds() {
    let doc = view().title_doc(&disc_entry(Some("2.76")));
    assert_eq!(
        doc.base
            .as_ref()
            .and_then(|b| b.shipped_firmware.as_deref()),
        Some("2.76")
    );
    let json = serde_json::to_string(&doc).expect("serialise");
    assert!(json.contains("\"shipped_firmware\":\"2.76\""), "{json}");
}

#[test]
fn a_base_that_ships_nothing_emits_no_shipped_firmware_key() {
    let doc = view().title_doc(&disc_entry(None));
    let json = serde_json::to_string(&doc).expect("serialise");
    assert!(!json.contains("shipped_firmware"), "{json}");
}

#[test]
fn the_detail_report_prints_the_shipped_version_beside_the_floor() {
    let rendered = render_title_detail(&view().title_doc(&disc_entry(Some("2.76"))));
    assert!(rendered.contains("  needs fw   02.7600\n"), "{rendered}");
    assert!(rendered.contains("  ships fw   2.76\n"), "{rendered}");

    let rendered = render_title_detail(&view().title_doc(&disc_entry(None)));
    assert!(!rendered.contains("ships fw"), "{rendered}");
}
