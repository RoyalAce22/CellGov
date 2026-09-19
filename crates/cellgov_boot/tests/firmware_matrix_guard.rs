//! Guards the firmware matrix against the title manifests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::TitleRegistry;
use cellgov_lv2::archive::{self, FirmwareRole, FIRMWARE};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn unexplained_priority_one(
    firmware: &BTreeMap<String, (u64, FirmwareRole)>,
    declared: &BTreeSet<String>,
) -> Vec<String> {
    firmware
        .iter()
        .filter(|&(fw, &(priority, role))| {
            priority == 1 && role != FirmwareRole::CensusReference && !declared.contains(fw)
        })
        .map(|(fw, _)| fw.clone())
        .collect()
}

#[test]
fn declared_cells_and_census_are_exactly_the_priority_one_rows() {
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
    let mut declared_firmware: BTreeSet<String> = BTreeSet::new();
    let mut missing: Vec<String> = Vec::new();
    let mut not_first: Vec<String> = Vec::new();
    for manifest in registry.iter() {
        for cell in &manifest.matrix {
            declared += 1;
            declared_firmware.insert(cell.key.fw.clone());
            match firmware.get(&cell.key.fw) {
                None => missing.push(format!(
                    "{} declares fw {}",
                    manifest.short_name, cell.key.fw
                )),
                Some(&(1, _)) => {}
                Some(&(priority, _)) => not_first.push(format!(
                    "{} declares fw {}, whose row has priority {priority}",
                    manifest.short_name, cell.key.fw
                )),
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
    assert!(
        not_first.is_empty(),
        "docs/lv2/tables/firmware.tsv gives priority 1 to every firmware a declared cell composes, \
         and these rows do not carry it:\n  {}",
        not_first.join("\n  ")
    );
    let unexplained = unexplained_priority_one(&firmware, &declared_firmware);
    assert!(
        unexplained.is_empty(),
        "docs/lv2/tables/firmware.tsv gives priority 1 only to firmware a declared cell composes and \
         the census reference; these rows have no such reason:\n  {}",
        unexplained.join("\n  ")
    );
    let probe = BTreeMap::from([
        ("1.50".to_string(), (1, FirmwareRole::None)),
        ("3.55".to_string(), (1, FirmwareRole::CensusReference)),
        ("4.93".to_string(), (1, FirmwareRole::Final)),
    ]);
    let probe_declared = BTreeSet::from(["1.50".to_string()]);
    assert_eq!(
        unexplained_priority_one(&probe, &probe_declared),
        ["4.93".to_string()]
    );
}
