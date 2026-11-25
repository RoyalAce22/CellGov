//! Reproduces the committed repository read log by booting each logged
//! firmware's stored kernel from the operator-owned installed firmware.
//!
//! With `CELLGOV_REGENERATE=1` set, the test writes the re-extracted
//! table over the committed one and compares nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_lv1_archive::{self as archive, REPOSITORY_LOGGED_FIRMWARE, REPOSITORY_READ};
use cellgov_lv2_archive as lv2_archive;
use cellgov_testkit::scratch::scratch_labeled;

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The PUP SHA-256 and the stored kernel of each installed firmware
/// entry that stores one, keyed by version.
fn stored_kernels(root: &Path) -> BTreeMap<String, (String, PathBuf)> {
    let layout = StoreLayout::new(root.join(DEFAULT_VFS_ROOT));
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records)
        .unwrap_or_else(|e| panic!("read firmware install records {}: {e}", records.display()));
    let mut kernels = BTreeMap::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read an entry of {}: {e}", records.display()))
            .path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".install.toml"))
        {
            continue;
        }
        let record = InstallRecord::parse(&read(&path))
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let Some(kernel) = record.core_os.as_ref().and_then(|c| c.kernel.as_ref()) else {
            continue;
        };
        let entry_dir = layout.resolve_store_path(&record.artifact.store_path);
        kernels.insert(
            record.artifact.version.clone(),
            (record.source.sha256.to_hex(), kernel.path_in(&entry_dir)),
        );
    }
    kernels
}

#[test]
fn firmware_boots_reextract_byte_identical_repository_reads() {
    let root = installed_firmware::workspace_root();
    let committed = root.join("docs/lv1").join(REPOSITORY_READ.file());
    let stored = stored_kernels(&root);
    let output = scratch_labeled("repository_census_firmware");

    assert_eq!(REPOSITORY_LOGGED_FIRMWARE.len(), 4);
    for fw in REPOSITORY_LOGGED_FIRMWARE {
        let (pup_sha256, kernel) = stored.get(*fw).unwrap_or_else(|| {
            panic!("firmware {fw}: no installed entry stores a kernel; install that firmware")
        });
        let result = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["dev", "repository-census"])
            .arg(kernel)
            .args(["--fw", fw, "--pup-sha256", pup_sha256])
            .arg("--output-dir")
            .arg(output.as_ref())
            .current_dir(&root)
            .output()
            .expect("run repository-census");
        assert!(
            result.status.success(),
            "repository-census firmware {fw} from {} failed:\n{}",
            kernel.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let extracted = read(&output.join(REPOSITORY_READ.file()));
    lv2_archive::parse(&REPOSITORY_READ, &extracted).expect("the re-extracted table parses");
    if std::env::var_os("CELLGOV_REGENERATE").is_some_and(|v| v == "1") {
        std::fs::write(&committed, &extracted)
            .unwrap_or_else(|e| panic!("write {}: {e}", committed.display()));
        return;
    }
    assert_eq!(
        extracted,
        read(&committed),
        "{} changed during firmware re-extraction; regenerate with:\n  {}",
        REPOSITORY_READ.file(),
        archive::REPOSITORY_REGENERATE
    );
}
