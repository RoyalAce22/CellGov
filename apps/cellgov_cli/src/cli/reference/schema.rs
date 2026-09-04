//! The `--format json` documents, rendered from their own `Serialize`
//! impls.

use crate::cli::store::read::model::{
    AnchorDoc, BaseDoc, DivergenceDoc, FirmwareDoc, FirmwareListDoc, StatusDoc, TitleDoc,
    TitleListDoc, UpdateDoc, VerifiedEntryDoc, VerifyDoc, STORE_FORMAT_VERSION,
};

/// A SHA-256 as a document spells one: 64 lowercase hex digits.
const SAMPLE_SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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
    FirmwareDoc {
        version: "4.93".to_string(),
        entry_dir: "firmware/4.93".to_string(),
        record: Some(".cellgov/installs/firmware/4.93.toml".to_string()),
        pup_sha256: SAMPLE_SHA.to_string(),
        image_version: Some("0x0004009300000000".to_string()),
        modules: Some(370),
        manifest_error: None,
    }
}

fn title_doc() -> TitleDoc {
    TitleDoc {
        title_id: "NPUA80001".to_string(),
        short_name: Some("flow".to_string()),
        display_name: Some("flOw".to_string()),
        base: Some(BaseDoc {
            app_ver: "01.00".to_string(),
            dir: "dev_hdd0/game/NPUA80001".to_string(),
            tree: "game".to_string(),
            distribution: "psn-hdd".to_string(),
            source_sha256: SAMPLE_SHA.to_string(),
            record: Some(".cellgov/installs/NPUA80001/base.toml".to_string()),
        }),
        ships_in_firmware: false,
        updates: vec![UpdateDoc {
            version: "1.02".to_string(),
            dir: "dev_hdd0/game/NPUA80001".to_string(),
            source_sha256: SAMPLE_SHA.to_string(),
            min_system_ver: Some("03.5500".to_string()),
            record: Some(".cellgov/installs/NPUA80001/1.02.toml".to_string()),
        }],
        anchors: vec![AnchorDoc {
            fw: "4.93".to_string(),
            game_ver: Some("base".to_string()),
            expect: "frontier".to_string(),
            reference: true,
            recorded: true,
            installed: true,
        }],
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
