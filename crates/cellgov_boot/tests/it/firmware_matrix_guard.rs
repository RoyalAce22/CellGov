//! Guards the firmware matrix against the title manifests: priority 1
//! is exactly the reference firmware CellGov measures every title at and the
//! census reference, and every firmware a declared cell composes has a
//! row.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::{TitleRegistry, REFERENCE_FIRMWARE};
use cellgov_lv2_archive::{self as archive, FirmwareRole, FIRMWARE};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The priority-1 rows that are neither the reference firmware nor the
/// census reference.
fn unexplained_priority_one(firmware: &BTreeMap<String, (u64, FirmwareRole)>) -> Vec<String> {
    firmware
        .iter()
        .filter(|&(fw, &(priority, role))| {
            priority == 1 && role != FirmwareRole::CensusReference && fw != REFERENCE_FIRMWARE
        })
        .map(|(fw, _)| fw.clone())
        .collect()
}

#[test]
fn the_reference_firmware_and_the_census_are_exactly_the_priority_one_rows() {
    let root = workspace_root();
    let text = std::fs::read_to_string(root.join("docs/lv2").join(FIRMWARE.file()))
        .unwrap_or_else(|e| panic!("read {}: {e}", FIRMWARE.file()));
    let table = archive::parse(&FIRMWARE, &text).unwrap_or_else(|e| panic!("{e}"));
    let rows = archive::firmware_rows(&table);
    archive::check_firmware_rows(&rows).unwrap_or_else(|e| panic!("{e}"));
    let firmware: BTreeMap<String, (u64, FirmwareRole)> = rows
        .into_iter()
        .map(|row| (row.fw, (row.priority, row.role)))
        .collect();

    let registry = TitleRegistry::scan_dir(&root.join("title_manifests"))
        .unwrap_or_else(|e| panic!("title registry: {e}"));
    let mut declared = 0usize;
    let mut missing: Vec<String> = Vec::new();
    for manifest in registry.iter() {
        for cell in &manifest.matrix {
            declared += 1;
            if !firmware.contains_key(&cell.key.fw) {
                missing.push(format!(
                    "{} declares fw {}",
                    manifest.short_name, cell.key.fw
                ));
            }
        }
    }
    assert!(
        declared >= 5,
        "gate went vacuous: only {declared} declared cell(s) read from title_manifests/"
    );
    assert!(
        missing.is_empty(),
        "title manifests declare firmware versions docs/lv2/tables/firmware.tsv has no row for:\n  {}",
        missing.join("\n  ")
    );
    assert_eq!(
        firmware.get(REFERENCE_FIRMWARE),
        Some(&(1, FirmwareRole::Final)),
        "docs/lv2/tables/firmware.tsv gives the reference firmware {REFERENCE_FIRMWARE} \
         priority 1 and the role final"
    );
    assert!(
        firmware
            .values()
            .any(|&(priority, role)| priority == 1 && role == FirmwareRole::CensusReference),
        "docs/lv2/tables/firmware.tsv gives the census reference priority 1"
    );
    let unexplained = unexplained_priority_one(&firmware);
    assert!(
        unexplained.is_empty(),
        "docs/lv2/tables/firmware.tsv gives priority 1 only to the reference firmware and the \
         census reference; these rows have no such reason:\n  {}",
        unexplained.join("\n  ")
    );
    let probe = BTreeMap::from([
        ("1.50".to_string(), (1, FirmwareRole::None)),
        ("3.55".to_string(), (1, FirmwareRole::CensusReference)),
        (REFERENCE_FIRMWARE.to_string(), (1, FirmwareRole::Final)),
    ]);
    assert_eq!(unexplained_priority_one(&probe), ["1.50".to_string()]);
}
