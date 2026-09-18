//! Guards committed boot anchors against the LV2 archive's PUP table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_compare::BootSummary;
use cellgov_lv2::archive::{self, PUP};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn find_anchor_summaries(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("read entry under {}: {error}", dir.display()))
            .path();
        if path.is_dir() {
            find_anchor_summaries(&path, out);
        } else if path
            .file_name()
            .is_some_and(|name| name == "boot_summary.json")
            && path.components().any(|part| part.as_os_str() == "anchors")
        {
            out.push(path);
        }
    }
}

#[test]
fn committed_anchors_name_a_pup_row_with_the_same_firmware() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs/lv2").join(PUP.file()))
        .unwrap_or_else(|error| panic!("read {}: {error}", PUP.file()));
    let table = archive::parse(&PUP, &text).unwrap_or_else(|error| panic!("{error}"));
    let pups: BTreeMap<String, (String, String)> = archive::pup_rows(&table)
        .into_iter()
        .map(|row| (row.pup_sha256, (row.fw, row.image_version)))
        .collect();

    let mut paths = Vec::new();
    find_anchor_summaries(&root.join("tests/fixtures"), &mut paths);
    paths.sort();
    let mut identities = 0usize;
    // The title-harness contract requires every committed anchor to state its firmware.
    let mut unidentified = Vec::new();
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let summary: BootSummary = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        let Some(firmware) = summary.identity.firmware else {
            unidentified.push(path.display().to_string());
            continue;
        };
        identities += 1;
        match pups.get(&firmware.pup_sha256) {
            None => missing.push(format!(
                "{} names fw {} PUP {}",
                path.display(),
                firmware.version,
                firmware.pup_sha256
            )),
            Some((fw, image_version))
                if fw != &firmware.version || image_version != &firmware.image_version =>
            {
                mismatched.push(format!(
                    "{} names fw {} image {}, but pup.tsv has fw {} image {}",
                    path.display(),
                    firmware.version,
                    firmware.image_version,
                    fw,
                    image_version
                ));
            }
            Some(_) => {}
        }
    }
    assert!(
        unidentified.is_empty(),
        "committed anchors without a firmware identity:\n  {}",
        unidentified.join("\n  ")
    );
    assert!(
        identities > 0,
        "no committed anchor carries a firmware identity"
    );
    assert!(
        missing.is_empty(),
        "committed anchors name PUPs absent from docs/lv2/pup.tsv:\n  {}",
        missing.join("\n  ")
    );
    assert!(
        mismatched.is_empty(),
        "committed anchors disagree with docs/lv2/pup.tsv:\n  {}",
        mismatched.join("\n  ")
    );
}
