//! Reproduces the committed LV2 census from the operator-owned installed firmware.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_lv2_archive::{self as archive, KernelRow, KERNEL, PUP};
use cellgov_testkit::scratch::scratch_labeled;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// One installed firmware entry's stored kernel.
struct StoredKernel {
    fw: String,
    pup_sha256: String,
    path: PathBuf,
}

/// The stored kernel of every installed firmware entry that has one.
fn stored_kernels(root: &Path) -> Vec<StoredKernel> {
    let layout = StoreLayout::new(root.join(DEFAULT_VFS_ROOT));
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records)
        .unwrap_or_else(|e| panic!("read firmware install records {}: {e}", records.display()));
    let mut kernels = Vec::new();
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
        kernels.push(StoredKernel {
            fw: record.artifact.version.clone(),
            pup_sha256: record.source.sha256.to_hex(),
            path: kernel.path_in(&entry_dir),
        });
    }
    kernels
}

/// The kernel a `kernel.tsv` row re-extracts from.
///
/// - The store entry installed from the row's PUP.
/// - Else `$CELLGOV_DUMPS_DIR/lv2-census/<pup sha256>/lv2_kernel.self`,
///   for a PUP the store does not hold. To fill it, install that PUP
///   into a scratch store with `cellgov firmware install <PUP> --output
///   <dir>`, `CELLGOV_KEYS` naming the vault, then copy the entry's
///   `core_os/lv2_kernel.self` there.
/// - Else the store entry of the row's firmware version, installed from
///   another PUP. The re-extracted `kernel.tsv` row names the kernel's
///   SHA-256, so the comparison below fails on a kernel that differs.
///
/// # Panics
///
/// When none of the three holds a kernel, naming the firmware.
fn kernel_for(row: &KernelRow, fw: &str, stored: &[StoredKernel]) -> PathBuf {
    if let Some(k) = stored.iter().find(|k| k.pup_sha256 == row.pup_sha256) {
        return k.path.clone();
    }
    if let Some(dumps) = std::env::var_os("CELLGOV_DUMPS_DIR") {
        let supplied = PathBuf::from(dumps)
            .join("lv2-census")
            .join(&row.pup_sha256)
            .join("lv2_kernel.self");
        if supplied.is_file() {
            return supplied;
        }
    }
    if let Some(k) = stored.iter().find(|k| k.fw == fw) {
        return k.path.clone();
    }
    panic!(
        "firmware {fw} (PUP {}): no installed entry stores a kernel for it. Install that \
         firmware, or place its lv2_kernel.self under $CELLGOV_DUMPS_DIR/lv2-census/{}/",
        row.pup_sha256, row.pup_sha256
    )
}

#[test]
fn firmware_reextracts_byte_identical_census_rows() {
    let root = workspace_root();
    let committed = root.join("docs/lv2");
    let pup_table = archive::parse(&PUP, &read(&committed.join(PUP.file())))
        .expect("parse committed PUP table");
    let firmware_by_pup: BTreeMap<String, String> = archive::pup_rows(&pup_table)
        .into_iter()
        .map(|row| (row.pup_sha256, row.fw))
        .collect();
    let kernel_table = archive::parse(&KERNEL, &read(&committed.join(KERNEL.file())))
        .expect("parse committed kernel table");
    let kernels = archive::kernel_rows(&kernel_table);
    let stored = stored_kernels(&root);
    let output = scratch_labeled("lv2_census_firmware");

    for kernel in &kernels {
        let fw = firmware_by_pup
            .get(&kernel.pup_sha256)
            .unwrap_or_else(|| panic!("kernel row names no PUP: {}", kernel.pup_sha256));
        let source = kernel_for(kernel, fw, &stored);
        let result = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["dev", "lv2-census"])
            .arg(&source)
            .args(["--fw", fw, "--pup-sha256", &kernel.pup_sha256])
            .arg("--output-dir")
            .arg(output.as_ref())
            .current_dir(&root)
            .output()
            .expect("run lv2-census");
        assert!(
            result.status.success(),
            "lv2-census firmware {fw} ({}) from {} failed:\n{}",
            kernel.pup_sha256,
            source.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    // Name the firmware of every row that moved before comparing whole
    // files, which says only that something did.
    let extracted = archive::parse(&KERNEL, &read(&output.join(KERNEL.file())))
        .expect("parse re-extracted kernel table");
    let extracted: BTreeMap<String, KernelRow> = archive::kernel_rows(&extracted)
        .into_iter()
        .map(|row| (row.pup_sha256.clone(), row))
        .collect();
    let moved: Vec<String> = kernels
        .iter()
        .filter(|row| extracted.get(&row.pup_sha256) != Some(row))
        .map(|row| {
            format!(
                "firmware {} (PUP {})",
                firmware_by_pup[&row.pup_sha256], row.pup_sha256
            )
        })
        .collect();
    assert!(
        moved.is_empty(),
        "kernel.tsv rows that did not re-extract byte-identically: {moved:?}"
    );

    for file in [
        KERNEL.file(),
        archive::STUB.file(),
        archive::SUBENTRY.file(),
        archive::CAPABILITY_GATE.file(),
    ] {
        assert_eq!(
            read(&output.join(&file)),
            read(&committed.join(&file)),
            "{file} changed during firmware re-extraction"
        );
    }
    let census_files: BTreeSet<String> = kernels
        .iter()
        .map(|kernel| archive::census_file(&firmware_by_pup[&kernel.pup_sha256]))
        .collect();
    for file in census_files {
        assert_eq!(
            read(&output.join(&file)),
            read(&committed.join(&file)),
            "{file} changed during firmware re-extraction"
        );
    }
}
