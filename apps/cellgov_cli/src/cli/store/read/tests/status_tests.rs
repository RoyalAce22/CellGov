use super::*;

use crate::cli::store::read::model::{
    AnchorDoc, BaseDoc, FirmwareDoc, NO_VERSION_KEY, STORE_FORMAT_VERSION,
};

/// Placeholder identity: these cases build every document by hand and
/// name no installed corpus.
const TITLE_ID: &str = "TEST00000";

fn firmware(version: &str) -> FirmwareDoc {
    FirmwareDoc {
        version: version.to_string(),
        entry_dir: format!("firmware/{version}"),
        record: Some(format!(".cellgov/installs/firmware/{version}.install.toml")),
        pup_sha256: "ab".repeat(32),
        image_version: Some("0x0004008200000000".to_string()),
        modules: Some(412),
        manifest_error: None,
    }
}

fn base() -> BaseDoc {
    BaseDoc {
        version: "01.00".to_string(),
        version_key: Some("app_ver".to_string()),
        param_sfo_error: None,
        dir: format!("dev_hdd0/game/{TITLE_ID}"),
        tree: "game".to_string(),
        distribution: "psn-hdd".to_string(),
        source_sha256: "cd".repeat(32),
        system_ver: None,
        record: Some(format!(
            ".cellgov/installs/titles/{TITLE_ID}/base.install.toml"
        )),
    }
}

fn cell(fw: &str, recorded: bool, installed: bool) -> AnchorDoc {
    AnchorDoc {
        fw: fw.to_string(),
        game_ver: Some("base".to_string()),
        expect: "frontier".to_string(),
        reference: true,
        recorded,
        installed,
    }
}

/// A cell of a title shipped inside the firmware: no game-version axis.
fn firmware_cell(fw: &str, recorded: bool, installed: bool) -> AnchorDoc {
    AnchorDoc {
        game_ver: None,
        ..cell(fw, recorded, installed)
    }
}

fn doc(firmware: Vec<FirmwareDoc>, titles: Vec<TitleDoc>) -> StatusDoc {
    StatusDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        store_bytes: 13_314_398_617,
        unreadable_paths: 0,
        firmware,
        titles,
    }
}

/// A title the registry declares as shipped inside the firmware.
fn firmware_shipped_title(anchors: Vec<AnchorDoc>) -> TitleDoc {
    TitleDoc {
        ships_in_firmware: true,
        ..title(None, anchors)
    }
}

fn title(base: Option<BaseDoc>, anchors: Vec<AnchorDoc>) -> TitleDoc {
    TitleDoc {
        title_id: TITLE_ID.to_string(),
        short_name: Some("synthetic".to_string()),
        display_name: Some("Synthetic".to_string()),
        ships_in_firmware: false,
        base,
        updates: Vec::new(),
        anchors,
    }
}

#[test]
fn the_report_leads_with_the_store_and_its_size() {
    let rendered = render(&doc(vec![firmware("4.91")], Vec::new()));
    assert!(rendered.starts_with("store  vfs  (12.4 GB)"), "{rendered}");
}

#[test]
fn a_size_the_walk_could_not_complete_is_reported_as_a_floor() {
    let mut partial = doc(vec![firmware("4.91")], Vec::new());
    partial.unreadable_paths = 3;
    let rendered = render(&partial);
    assert!(
        rendered.starts_with("store  vfs  (at least 12.4 GB; 3 path(s) could not be read)"),
        "{rendered}"
    );
}

#[test]
fn an_empty_machine_says_so_for_both_blocks() {
    let rendered = render(&doc(Vec::new(), Vec::new()));
    assert!(rendered.contains("firmware   none installed"), "{rendered}");
    assert!(
        rendered.contains("titles     none installed and none declared"),
        "{rendered}"
    );
}

#[test]
fn a_declared_title_with_nothing_installed_is_named_as_such() {
    let rendered = render(&doc(vec![firmware("4.91")], vec![title(None, Vec::new())]));
    assert!(
        rendered.contains("declared, nothing installed"),
        "{rendered}"
    );
}

#[test]
fn an_installed_base_is_summarised_by_its_distribution_and_version_under_its_key() {
    let rendered = render(&doc(
        vec![firmware("4.91")],
        vec![title(Some(base()), Vec::new())],
    ));
    assert!(
        rendered.contains("psn-hdd base app_ver 01.00"),
        "{rendered}"
    );
}

#[test]
fn a_base_whose_table_named_no_version_is_labelled_rather_than_blank() {
    let unversioned = BaseDoc {
        version: String::new(),
        ..base()
    };
    let rendered = render(&doc(
        vec![firmware("4.91")],
        vec![title(Some(unversioned), Vec::new())],
    ));
    assert!(
        rendered.contains(&format!("psn-hdd base {NO_VERSION_KEY}")),
        "{rendered}"
    );
}

#[test]
fn every_declared_cell_appears_with_its_anchor_state() {
    let rendered = render(&doc(
        vec![firmware("4.91")],
        vec![title(
            Some(base()),
            vec![cell("4.91", true, true), cell("3.55", false, false)],
        )],
    ));
    assert!(rendered.contains("fw 4.91 x base   recorded"), "{rendered}");
    assert!(rendered.contains("fw 3.55 x base   none"), "{rendered}");
}

#[test]
fn a_title_shipped_inside_the_firmware_is_not_reported_as_nothing_installed() {
    let rendered = render(&doc(
        vec![firmware("4.93")],
        vec![firmware_shipped_title(vec![firmware_cell(
            "4.93", false, true,
        )])],
    ));
    assert!(rendered.contains("ships inside the firmware"), "{rendered}");
    assert!(
        !rendered.contains("declared, nothing installed"),
        "{rendered}"
    );
}

#[test]
fn a_title_whose_firmware_is_absent_is_not_reported_as_shipped_and_present() {
    let rendered = render(&doc(
        Vec::new(),
        vec![firmware_shipped_title(vec![firmware_cell(
            "4.93", false, false,
        )])],
    ));
    assert!(
        rendered.contains("ships inside the firmware, none of whose versions is installed"),
        "{rendered}"
    );
}

#[test]
fn a_machine_with_no_firmware_is_pointed_at_installing_one() {
    let hint = next_step(&doc(Vec::new(), vec![title(Some(base()), Vec::new())]));
    assert_eq!(
        hint.as_deref(),
        Some("cellgov firmware install <PS3UPDAT.PUP>")
    );
}

#[test]
fn a_machine_with_no_title_is_pointed_at_installing_one() {
    let hint = next_step(&doc(vec![firmware("4.91")], vec![title(None, Vec::new())]));
    assert_eq!(hint.as_deref(), Some("cellgov title install <PKG|ISO>"));
}

#[test]
fn a_bootable_firmware_shipped_title_is_benched_rather_than_installed_over() {
    let hint = next_step(&doc(
        vec![firmware("4.93")],
        vec![firmware_shipped_title(vec![firmware_cell(
            "4.93", false, true,
        )])],
    ))
    .expect("the firmware-shipped cell is bootable and unrecorded");
    assert_eq!(
        hint,
        "cellgov boot bench --title synthetic --fw 4.93   (no anchor yet)"
    );
}

#[test]
fn the_one_bootable_cell_with_no_anchor_is_the_next_step() {
    let hint = next_step(&doc(
        vec![firmware("4.91")],
        vec![title(
            Some(base()),
            vec![cell("4.91", false, true), cell("3.55", false, false)],
        )],
    ))
    .expect("one unrecorded cell is bootable here");
    assert!(
        hint.contains("boot bench --title synthetic --fw 4.91 --game-ver base"),
        "{hint}"
    );
}

#[test]
fn two_bootable_cells_with_no_anchor_suggest_nothing() {
    assert!(next_step(&doc(
        vec![firmware("4.91"), firmware("3.55")],
        vec![title(
            Some(base()),
            vec![cell("4.91", false, true), cell("3.55", false, true)],
        )],
    ))
    .is_none());
}

#[test]
fn a_fully_recorded_machine_suggests_nothing() {
    assert!(next_step(&doc(
        vec![firmware("4.91")],
        vec![title(Some(base()), vec![cell("4.91", true, true)])],
    ))
    .is_none());
}

#[test]
fn cells_that_cannot_be_booted_here_are_not_suggested() {
    assert!(next_step(&doc(
        vec![firmware("4.91")],
        vec![title(
            Some(base()),
            vec![cell("3.55", false, false), cell("4.20", false, false)],
        )],
    ))
    .is_none());
}
