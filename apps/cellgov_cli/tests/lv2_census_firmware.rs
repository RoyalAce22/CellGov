//! Reproduces the committed LV2 census from the operator-owned installed firmware.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use cellgov_lv2::archive::{self, KERNEL, PUP};
use cellgov_testkit::scratch::scratch_labeled;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn firmware_reextracts_byte_identical_census_rows() {
    let dumps = std::env::var_os("CELLGOV_DUMPS_DIR")
        .map(PathBuf::from)
        .expect("installed-firmware-tests requires CELLGOV_DUMPS_DIR");
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
    let output = scratch_labeled("lv2_census_firmware");

    for kernel in &kernels {
        let fw = firmware_by_pup
            .get(&kernel.pup_sha256)
            .unwrap_or_else(|| panic!("kernel row names no PUP: {}", kernel.pup_sha256));
        let elf = dumps
            .join("lv2-census")
            .join(&kernel.pup_sha256)
            .join("lv2_kernel.elf");
        assert!(
            elf.is_file(),
            "missing firmware fixture {}; every kernel.tsv row must have one",
            elf.display()
        );
        let result = Command::new(env!("CARGO_BIN_EXE_cellgov"))
            .args(["dev", "lv2-census"])
            .arg(&elf)
            .args(["--fw", fw, "--pup-sha256", &kernel.pup_sha256])
            .arg("--output-dir")
            .arg(output.as_ref())
            .output()
            .expect("run lv2-census");
        assert!(
            result.status.success(),
            "lv2-census {} failed:\n{}",
            kernel.pup_sha256,
            String::from_utf8_lossy(&result.stderr)
        );
    }

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
