//! The `--format json` documents, rendered from their own `Serialize`
//! impls.
//!
//! [`StoreLayout`] resolves every record path and entry directory a
//! sample names, the way the read commands resolve the real ones.

use std::path::Path;

use cellgov_install::store::{Artifact, StoreLayout, TitleId, TitleTree, VersionKey};

use crate::cli::store::read::model::{
    store_rel, AnchorDoc, BaseDoc, DivergenceDoc, FirmwareDoc, FirmwareListDoc, StatusDoc,
    TitleDoc, TitleListDoc, UpdateDoc, VerifiedEntryDoc, VerifyDoc, STORE_FORMAT_VERSION,
};

/// A SHA-256 as a document spells one: 64 lowercase hex digits.
const SAMPLE_SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The store root of every sample path.
const SAMPLE_ROOT: &str = "vfs";

const SAMPLE_FIRMWARE_VERSION: &str = "4.93";
const SAMPLE_TITLE_ID: &str = "NPUA80001";
const SAMPLE_UPDATE_VERSION: &str = "1.02";

fn layout() -> StoreLayout {
    StoreLayout::new(SAMPLE_ROOT)
}

/// A store path, relative to [`SAMPLE_ROOT`], as a document spells it.
fn rel(path: &Path) -> String {
    store_rel(Path::new(SAMPLE_ROOT), path)
}

/// Where the store files the record for `artifact`, as a document
/// spells it.
fn record(artifact: &Artifact) -> Option<String> {
    Some(rel(&layout().record_path(artifact)))
}

fn sample_title_id() -> TitleId {
    TitleId::new(SAMPLE_TITLE_ID).expect("invariant: the sample title id is a store key")
}

fn sample_version(version: &str) -> VersionKey {
    VersionKey::new(version).expect("invariant: the sample versions are store keys")
}

/// Every versioned document, each under the commands that emit it.
pub(crate) fn render() -> String {
    let mut s = String::new();
    push(&mut s, "`status`", &status_doc());
    push(
        &mut s,
        "`firmware list`, `firmware show`",
        &firmware_list_doc(),
    );
    push(&mut s, "`title list`, `title show`", &title_list_doc());
    push(&mut s, "`firmware verify`, `title verify`", &verify_doc());
    s
}

/// One document under its heading, as a fenced JSON block.
fn push<T: serde::Serialize>(out: &mut String, commands: &str, doc: &T) {
    let body = serde_json::to_string_pretty(doc).expect(
        "invariant: the read documents are derived Serialize over scalars, String, Option \
         and Vec -- no map key that is not a string, and no impl that can refuse",
    );
    out.push_str(&format!("{commands}:\n\n```json\n{body}\n```\n\n"));
}

fn firmware_doc() -> FirmwareDoc {
    let artifact = Artifact::Firmware {
        version: sample_version(SAMPLE_FIRMWARE_VERSION),
    };
    FirmwareDoc {
        version: SAMPLE_FIRMWARE_VERSION.to_string(),
        entry_dir: rel(&layout().entry_dir(&artifact)),
        record: record(&artifact),
        pup_sha256: SAMPLE_SHA.to_string(),
        image_version: Some("0x0004009300000000".to_string()),
        modules: Some(370),
        manifest_error: None,
    }
}

fn title_doc() -> TitleDoc {
    let base = Artifact::TitleBase {
        title_id: sample_title_id(),
    };
    let update = Artifact::TitleUpdate {
        title_id: sample_title_id(),
        version: sample_version(SAMPLE_UPDATE_VERSION),
    };
    TitleDoc {
        title_id: SAMPLE_TITLE_ID.to_string(),
        short_name: Some("flow".to_string()),
        display_name: Some("flOw".to_string()),
        base: Some(BaseDoc {
            version: "01.00".to_string(),
            version_key: Some("app_ver".to_string()),
            param_sfo_error: None,
            // The PKG installer writes a base as the live
            // `dev_hdd0/game/<id>` mount directory, which the layout
            // names no entry for.
            dir: format!("dev_hdd0/game/{SAMPLE_TITLE_ID}"),
            tree: TitleTree::Game.dir_name().to_string(),
            distribution: "psn-hdd".to_string(),
            source_sha256: SAMPLE_SHA.to_string(),
            system_ver: Some("01.5000".to_string()),
            record: record(&base),
        }),
        ships_in_firmware: false,
        updates: vec![UpdateDoc {
            version: SAMPLE_UPDATE_VERSION.to_string(),
            version_key: Some("app_ver".to_string()),
            param_sfo_error: None,
            dir: rel(&layout().entry_dir(&update).join(TitleTree::Game.dir_name())),
            source_sha256: SAMPLE_SHA.to_string(),
            min_system_ver: Some("03.5500".to_string()),
            system_ver: Some("03.5500".to_string()),
            record: record(&update),
        }],
        anchors: vec![
            AnchorDoc {
                fw: "1.50".to_string(),
                game_ver: Some("base".to_string()),
                expect: "frontier".to_string(),
                reference: true,
                recorded: true,
                installed: true,
            },
            AnchorDoc {
                fw: "4.93".to_string(),
                game_ver: Some("base".to_string()),
                expect: "frontier".to_string(),
                reference: false,
                recorded: true,
                installed: true,
            },
        ],
    }
}

fn status_doc() -> StatusDoc {
    StatusDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        store_bytes: 21_474_836_480,
        unreadable_paths: 0,
        firmware: vec![firmware_doc()],
        titles: vec![title_doc()],
    }
}

fn firmware_list_doc() -> FirmwareListDoc {
    FirmwareListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        firmware: vec![firmware_doc()],
    }
}

fn title_list_doc() -> TitleListDoc {
    TitleListDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        titles: vec![title_doc()],
    }
}

fn verify_doc() -> VerifyDoc {
    VerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        subject: "NPUA80001".to_string(),
        entries: vec![VerifiedEntryDoc {
            entry: "base".to_string(),
            matched: 128,
            divergences: vec![DivergenceDoc {
                path: "dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN".to_string(),
                kind: "modified".to_string(),
                expected: Some(SAMPLE_SHA.to_string()),
                found: Some(SAMPLE_SHA.to_string()),
                reason: None,
            }],
        }],
        clean: false,
    }
}
