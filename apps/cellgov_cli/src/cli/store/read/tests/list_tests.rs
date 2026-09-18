use super::*;

use crate::cli::store::read::model::{
    AnchorDoc, BaseDoc, CoreOsDoc, UpdateDoc, NO_VERSION_KEY, STORE_FORMAT_VERSION,
};

/// Placeholder identity: these cases build every document by hand and
/// name no installed corpus.
const TITLE_ID: &str = "TEST00000";

fn title(short_name: Option<&str>, base: Option<BaseDoc>, updates: Vec<UpdateDoc>) -> TitleDoc {
    TitleDoc {
        title_id: TITLE_ID.to_string(),
        short_name: short_name.map(str::to_string),
        display_name: short_name.map(str::to_string),
        ships_in_firmware: false,
        base,
        updates,
        anchors: Vec::new(),
    }
}

fn base(version: &str) -> BaseDoc {
    BaseDoc {
        version: version.to_string(),
        version_key: None,
        param_sfo_error: None,
        dir: format!("dev_hdd0/game/{TITLE_ID}"),
        tree: "game".to_string(),
        distribution: "psn-hdd".to_string(),
        source_sha256: "ab".repeat(32),
        system_ver: None,
        shipped_firmware: None,
        record: Some(format!(
            ".cellgov/installs/titles/{TITLE_ID}/base.install.toml"
        )),
    }
}

fn update(version: &str) -> UpdateDoc {
    UpdateDoc {
        version: version.to_string(),
        version_key: None,
        param_sfo_error: None,
        dir: format!("titles/{TITLE_ID}/updates/{version}/game"),
        source_sha256: "cd".repeat(32),
        min_system_ver: None,
        system_ver: None,
        record: Some(format!(
            ".cellgov/installs/titles/{TITLE_ID}/update-{version}.install.toml"
        )),
    }
}

fn title_doc(titles: Vec<TitleDoc>) -> TitleListDoc {
    TitleListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        titles,
    }
}

#[test]
fn an_empty_store_says_so_rather_than_printing_a_bare_header() {
    let rendered = render_title_list(&title_doc(Vec::new()));
    assert!(
        rendered.contains("no title installed under vfs"),
        "{rendered}"
    );
    assert!(!rendered.contains("TITLE ID"), "{rendered}");

    let rendered = render_firmware_list(&FirmwareListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        firmware: Vec::new(),
    });
    assert!(
        rendered.contains("no firmware installed under vfs"),
        "{rendered}"
    );
}

#[test]
fn a_listed_title_names_its_base_and_every_update() {
    let rendered = render_title_list(&title_doc(vec![title(
        Some("synthetic"),
        Some(base("01.00")),
        vec![update("02.10"), update("02.51")],
    )]));
    assert!(rendered.contains(TITLE_ID), "{rendered}");
    assert!(rendered.contains("synthetic"), "{rendered}");
    assert!(rendered.contains("01.00"), "{rendered}");
    assert!(rendered.contains("02.10, 02.51"), "{rendered}");
}

#[test]
fn a_base_whose_table_named_no_version_is_labelled_rather_than_blank() {
    let rendered = render_title_list(&title_doc(vec![title(
        Some("synthetic"),
        Some(base("")),
        Vec::new(),
    )]));
    assert!(rendered.contains(NO_VERSION_KEY), "{rendered}");

    let rendered = render_title_detail(&title(Some("synthetic"), Some(base("")), Vec::new()));
    assert!(
        rendered.contains(&format!("base       {NO_VERSION_KEY} (psn-hdd, game tree)")),
        "{rendered}"
    );
}

#[test]
fn a_base_with_a_version_prints_it_and_not_the_no_version_label() {
    let rendered = render_title_detail(&title(Some("synthetic"), Some(base("01.00")), Vec::new()));
    assert!(
        rendered.contains("base       01.00 (psn-hdd, game tree)"),
        "{rendered}"
    );
    assert!(!rendered.contains(NO_VERSION_KEY), "{rendered}");
}

#[test]
fn a_declared_system_version_is_printed_per_entry_and_omitted_when_absent() {
    let silent = render_title_detail(&title(
        Some("synthetic"),
        Some(base("01.00")),
        vec![update("02.51")],
    ));
    assert!(!silent.contains("needs fw"), "{silent}");

    let mut declaring_base = base("01.00");
    declaring_base.system_ver = Some("01.5000".to_string());
    let mut declaring_update = update("02.51");
    declaring_update.system_ver = Some("03.5500".to_string());
    declaring_update.min_system_ver = Some("03.5000".to_string());
    let rendered = render_title_detail(&title(
        Some("synthetic"),
        Some(declaring_base),
        vec![declaring_update],
    ));
    assert!(rendered.contains("  needs fw   01.5000\n"), "{rendered}");
    assert!(rendered.contains("    needs fw 03.5500\n"), "{rendered}");
    assert!(
        rendered.contains("    min fw   03.5000\n"),
        "the metadata's claim stays its own line: {rendered}"
    );
}

#[test]
fn a_title_no_manifest_declares_is_flagged_as_an_orphan() {
    let rendered = render_title_list(&title_doc(vec![title(
        None,
        Some(base("01.00")),
        Vec::new(),
    )]));
    assert!(rendered.contains(NO_MANIFEST), "{rendered}");
    assert!(
        rendered.contains(&format!(
            "{TITLE_ID}: orphan -- no {}/*.toml declares this title",
            crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR
        )),
        "{rendered}"
    );
}

#[test]
fn updates_with_no_base_are_flagged_as_an_orphan() {
    let rendered = render_title_list(&title_doc(vec![title(
        Some("synthetic"),
        None,
        vec![update("02.51")],
    )]));
    assert!(
        rendered.contains("orphan") && rendered.contains("no base to patch"),
        "{rendered}"
    );
}

#[test]
fn a_declared_title_with_a_base_is_flagged_neither_way() {
    let rendered = render_title_list(&title_doc(vec![title(
        Some("synthetic"),
        Some(base("01.00")),
        vec![update("02.51")],
    )]));
    assert!(!rendered.contains("orphan"), "{rendered}");
}

#[test]
fn a_title_detail_names_each_cell_and_whether_it_has_an_anchor() {
    let mut doc = title(Some("synthetic"), Some(base("01.00")), Vec::new());
    doc.anchors = vec![
        AnchorDoc {
            fw: "4.91".to_string(),
            game_ver: Some("base".to_string()),
            expect: "frontier".to_string(),
            reference: true,
            recorded: true,
            installed: true,
        },
        AnchorDoc {
            fw: "3.55".to_string(),
            game_ver: Some("base".to_string()),
            expect: "probe".to_string(),
            reference: false,
            recorded: false,
            installed: false,
        },
    ];
    let rendered = render_title_detail(&doc);
    assert!(rendered.contains("fw 4.91 x base"), "{rendered}");
    assert!(rendered.contains("recorded, reference"), "{rendered}");
    assert!(rendered.contains("fw 3.55 x base"), "{rendered}");
    assert!(
        rendered.contains("no anchor") && rendered.contains("not installed here"),
        "{rendered}"
    );
}

fn firmware(modules: Option<usize>, manifest_error: Option<&str>) -> FirmwareDoc {
    FirmwareDoc {
        version: "4.91".to_string(),
        entry_dir: "firmware/4.91".to_string(),
        record: Some(".cellgov/installs/firmware/4.91.install.toml".to_string()),
        pup_sha256: "ef".repeat(32),
        image_version: None,
        modules,
        manifest_error: manifest_error.map(str::to_string),
        core_os: None,
    }
}

fn core_os(kernel: bool, omission: Option<&str>, files: usize) -> CoreOsDoc {
    CoreOsDoc {
        kernel: kernel.then(|| crate::cli::store::read::model::KernelDoc {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: "ab".repeat(32),
        }),
        omission: omission.map(str::to_string),
        files: (0..files)
            .map(|i| crate::cli::store::read::model::CoreOsFileDoc {
                name: format!("file{i}"),
                size: 16,
            })
            .collect(),
    }
}

#[test]
fn an_entry_whose_record_predates_the_kernel_reads_as_not_unpacked_never_as_an_error() {
    let rendered = render_firmware_detail(&firmware(Some(412), None));
    assert!(
        rendered.contains(&format!("kernel     {KERNEL_NOT_RECORDED}")),
        "{rendered}"
    );
    assert!(rendered.contains("--kernel-only"), "{rendered}");
    assert!(
        !rendered.contains("core os"),
        "no table to count: {rendered}"
    );
}

#[test]
fn a_stored_kernel_prints_its_path_its_digest_and_the_table_it_came_from() {
    let mut entry = firmware(Some(412), None);
    entry.core_os = Some(core_os(true, None, 25));
    let rendered = render_firmware_detail(&entry);
    assert!(
        rendered.contains(&format!(
            "kernel     core_os/lv2_kernel.self (as stored, sha256 {})",
            "ab".repeat(32)
        )),
        "{rendered}"
    );
    assert!(
        rendered.contains("core os    25 file(s) in the package table"),
        "{rendered}"
    );
}

#[test]
fn an_omitted_kernel_prints_why_and_still_counts_the_table_it_read() {
    let mut entry = firmware(Some(412), None);
    entry.core_os = Some(core_os(
        false,
        Some("CORE_OS_PACKAGE.pkg names no lv2_kernel.self among its 3 file(s)"),
        3,
    ));
    let rendered = render_firmware_detail(&entry);
    assert!(
        rendered.contains("kernel     not unpacked (CORE_OS_PACKAGE.pkg names no lv2_kernel.self"),
        "{rendered}"
    );
    assert!(rendered.contains("core os    3 file(s)"), "{rendered}");

    entry.core_os = Some(core_os(false, Some("update_files carries no package"), 0));
    let rendered = render_firmware_detail(&entry);
    assert!(!rendered.contains("core os"), "{rendered}");
}

#[test]
fn the_list_marks_which_versions_store_a_kernel() {
    let mut with = firmware(Some(412), None);
    with.core_os = Some(core_os(true, None, 25));
    let mut without = firmware(Some(400), None);
    without.version = "3.55".to_string();
    without.core_os = Some(core_os(false, Some("no package"), 0));
    let rendered = render_firmware_list(&FirmwareListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        firmware: vec![with, without, firmware(None, None)],
    });
    assert!(rendered.contains("KERNEL"), "{rendered}");
    let lines: Vec<&str> = rendered.lines().collect();
    assert!(
        lines[1].contains("4.91") && lines[1].contains("yes"),
        "{rendered}"
    );
    assert!(
        lines[2].contains("3.55") && !lines[2].contains("yes"),
        "{rendered}"
    );
    assert!(!lines[3].contains("yes"), "{rendered}");
}

#[test]
fn a_firmware_mount_with_no_manifest_reports_no_module_count() {
    let rendered = render_firmware_detail(&firmware(None, None));
    assert!(rendered.contains("modules    --"), "{rendered}");
    assert!(
        !rendered.contains("modules    0"),
        "an unreadable manifest must not read as a tree with no modules: {rendered}"
    );
}

#[test]
fn a_manifest_that_refused_to_load_names_why() {
    let rendered = render_firmware_detail(&firmware(
        None,
        Some("read vfs/firmware/4.91/dev_flash/firmware.toml: permission denied"),
    ));
    assert!(rendered.contains("permission denied"), "{rendered}");
}

/// A record path is a path a consumer joins onto the store root, so a
/// key that names none renders as absent.
#[test]
fn an_entry_whose_key_names_no_record_says_so_rather_than_printing_nothing() {
    let mut entry = firmware(Some(412), None);
    entry.record = None;
    let rendered = render_firmware_detail(&entry);
    assert!(rendered.contains(NO_RECORD), "{rendered}");
}
