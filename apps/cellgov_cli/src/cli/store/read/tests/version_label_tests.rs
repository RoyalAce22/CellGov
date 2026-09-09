use super::*;

use crate::cli::store::read::model::{BaseDoc, UpdateDoc, NO_VERSION_KEY, STORE_FORMAT_VERSION};

/// Placeholder identity: these cases build every document by hand and
/// name no installed corpus.
const TITLE_ID: &str = "TEST00000";

fn base(version: &str, key: Option<&str>, error: Option<&str>) -> BaseDoc {
    BaseDoc {
        version: version.to_string(),
        version_key: key.map(str::to_string),
        param_sfo_error: error.map(str::to_string),
        dir: format!("dev_hdd0/game/{TITLE_ID}"),
        tree: "game".to_string(),
        distribution: "psn-hdd".to_string(),
        source_sha256: "ab".repeat(32),
        system_ver: None,
        shipped_firmware: None,
        record: None,
    }
}

fn update(version: &str, key: Option<&str>, error: Option<&str>) -> UpdateDoc {
    UpdateDoc {
        version: version.to_string(),
        version_key: key.map(str::to_string),
        param_sfo_error: error.map(str::to_string),
        dir: format!("titles/{TITLE_ID}/updates/{version}/game"),
        source_sha256: "cd".repeat(32),
        min_system_ver: None,
        system_ver: None,
        record: None,
    }
}

fn title(base: Option<BaseDoc>, updates: Vec<UpdateDoc>) -> TitleDoc {
    TitleDoc {
        title_id: TITLE_ID.to_string(),
        short_name: Some("synthetic".to_string()),
        display_name: Some("Synthetic".to_string()),
        ships_in_firmware: false,
        base,
        updates,
        anchors: Vec::new(),
    }
}

fn list_of(title: TitleDoc) -> String {
    render_title_list(&TitleListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        titles: vec![title],
    })
}

/// The column a line's `needle` starts in.
fn column_of(rendered: &str, needle: &str) -> usize {
    rendered
        .lines()
        .find_map(|line| line.find(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not in:\n{rendered}"))
}

#[test]
fn a_base_prints_its_version_under_the_key_that_named_it() {
    let doc = title(Some(base("01.00", Some("app_ver"), None)), Vec::new());
    let detail = render_title_detail(&doc);
    assert!(
        detail.contains("base       app_ver 01.00 (psn-hdd, game tree)"),
        "{detail}"
    );
    assert!(!detail.contains("param.sfo"), "{detail}");
    assert!(list_of(doc).contains("app_ver 01.00"));
}

#[test]
fn a_version_under_its_longer_key_keeps_the_updates_column_in_line() {
    let rendered = list_of(title(
        Some(base("01.00", Some("sfo_version"), None)),
        vec![update("02.51", Some("app_ver"), None)],
    ));
    assert!(rendered.contains("sfo_version 01.00"), "{rendered}");
    assert_eq!(
        column_of(&rendered, "UPDATES"),
        column_of(&rendered, "02.51"),
        "{rendered}"
    );
}

#[test]
fn a_base_whose_table_did_not_confirm_the_record_prints_bare_and_names_why() {
    let detail = render_title_detail(&title(
        Some(base("01.00", None, Some("read PARAM.SFO: no such file"))),
        Vec::new(),
    ));
    assert!(
        detail.contains("base       01.00 (psn-hdd, game tree)"),
        "{detail}"
    );
    assert!(
        detail.contains("  param.sfo  read PARAM.SFO: no such file\n"),
        "{detail}"
    );
}

#[test]
fn an_update_heading_names_its_key_and_its_own_table_error() {
    let detail = render_title_detail(&title(
        Some(base("01.00", Some("app_ver"), None)),
        vec![
            update("02.10", Some("sfo_version"), None),
            update("02.51", None, Some("read PARAM.SFO: no such file")),
        ],
    ));
    assert!(detail.contains("  update sfo_version 02.10\n"), "{detail}");
    assert!(detail.contains("  update 02.51\n"), "{detail}");
    assert!(
        detail.contains("    param.sfo read PARAM.SFO: no such file\n"),
        "{detail}"
    );
    assert_eq!(detail.matches("param.sfo").count(), 1, "{detail}");
}

#[test]
fn the_updates_column_lists_bare_store_keys_whatever_named_them() {
    let rendered = list_of(title(
        Some(base("01.00", Some("app_ver"), None)),
        vec![
            update("02.10", Some("app_ver"), None),
            update("02.51", Some("sfo_version"), None),
        ],
    ));
    assert!(rendered.contains("02.10, 02.51"), "{rendered}");
}

#[test]
fn an_empty_version_prints_the_no_version_label_whatever_its_table_said() {
    let detail = render_title_detail(&title(
        Some(base("", None, Some("read PARAM.SFO: no such file"))),
        Vec::new(),
    ));
    assert!(
        detail.contains(&format!("base       {NO_VERSION_KEY} (psn-hdd, game tree)")),
        "{detail}"
    );
}
