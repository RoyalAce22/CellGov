//! Integration tests for the firmware install pipeline against a real
//! PS3UPDAT.PUP.
//!
//! The `cellgov` binary lives in a crate that depends on this one, so
//! the suite drives `install_pup` rather than the command: the store
//! root it is handed is what `cellgov firmware install --output`
//! names, and the firmware lands in the entry its own
//! `vsh/etc/version.txt` keys, beside whatever a title install put in
//! `dev_hdd0` / `dev_bdvd`.
//!
//! The PUP is operator-owned and read from the dump root
//! (`common/dumps.rs`); nothing in the repo ships one. This suite is
//! compiled only under the `firmware-dumps` feature, which declares
//! that fixture present: a missing one is a hard failure rather than
//! a skip.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::{Path, PathBuf};

use cellgov_install::firmware_install::{FirmwareInstallError, FirmwareInstallOutcome};
use cellgov_install::store::{Artifact, ArtifactKind, InstallRecord, StoreLayout, VersionKey};
use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;

#[path = "common/digests.rs"]
mod digests;
#[path = "common/dumps.rs"]
mod dumps;
#[path = "common/keys.rs"]
mod keys;
#[path = "common/scratch.rs"]
mod scratch;

/// The PUP under test.
///
/// # Panics
///
/// If the dump root holds no PUP.
fn locate_pup() -> PathBuf {
    let p = dumps::ps3updat_pup();
    assert!(
        p.is_file(),
        "firmware-dumps: no PUP at {}. Either drop a PS3UPDAT.PUP \
         there or point {} at a dump root holding one.",
        p.display(),
        dumps::ENV_DUMPS_DIR,
    );
    p
}

/// The vault is loaded from the operator's own location rather than
/// from the scratch store, where nothing was imported.
fn run_install(
    pup: &Path,
    store_root: &Path,
    force: bool,
) -> Result<FirmwareInstallOutcome, FirmwareInstallError> {
    let vault = cellgov_install::keys::KeyVault::load_from_path(Path::new(&keys::location()))
        .expect("the operator vault this suite declares");
    let pup_data = std::fs::read(pup).expect("read the PUP this suite located");
    cellgov_install::firmware_install::install_pup(&pup_data, &vault, store_root, force, &())
}

/// # Panics
///
/// If the install failed, naming the refusal.
fn assert_succeeded(result: Result<FirmwareInstallOutcome, FirmwareInstallError>, what: &str) {
    if let Err(e) = result {
        panic!("{what} failed: {e}");
    }
}

/// The single firmware entry under `vfs_root`, and the version keying
/// it.
///
/// # Panics
///
/// Unless exactly one version directory is there -- an install that
/// wrote two entries, or none, is a failure this suite must not read
/// past.
fn sole_entry(vfs_root: &Path) -> (String, PathBuf) {
    let layout = StoreLayout::new(vfs_root);
    let root = layout.firmware_root();
    let mut versions: Vec<String> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    versions.sort();
    assert_eq!(
        versions.len(),
        1,
        "expected exactly one firmware entry under {}, found {versions:?}",
        root.display(),
    );
    let version = versions.remove(0);
    let entry = layout.entry_dir(&Artifact::Firmware {
        version: VersionKey::new(&version).expect("the install wrote a usable version key"),
    });
    (version, entry)
}

/// Every module the committed digest table pins is present in `dir`.
///
/// "At least one file with a module extension" would pass an install
/// that wrote a single module. Each `decrypted_masked/<stem>` row is a
/// module `parity_firmware_decrypt` later decrypts out of an installed
/// firmware tree, so a fresh install has to produce all of them.
///
/// # Panics
///
/// If a pinned module is absent, or the table pins none -- a table
/// with no such row would make this check assert nothing.
fn assert_pinned_modules_present(dir: &Path) {
    let mut pinned = 0usize;
    for key in digests::table().keys() {
        let Some(stem) = key.strip_prefix("decrypted_masked/") else {
            continue;
        };
        let module = dir.join(format!("{stem}.sprx"));
        assert!(
            module.is_file(),
            "install populated {} but wrote no {}, pinned by the \
             committed digest table under {key}",
            dir.display(),
            module.display(),
        );
        pinned += 1;
    }
    assert!(
        pinned > 0,
        "the committed digest table pins no decrypted_masked/ row, so \
         this install asserted nothing about the modules it wrote"
    );
}

/// `sys/external` under a firmware entry, where the pinned modules land.
fn sys_external(entry: &Path) -> PathBuf {
    entry.join(FLASH_MOUNT).join("sys").join("external")
}

#[test]
fn install_keys_the_entry_on_the_version_the_extracted_tree_names() {
    let pup = locate_pup();
    let output = scratch::scratch_labeled("fw_happy");
    assert_succeeded(run_install(&pup, &output, false), "install");

    let (version, entry) = sole_entry(&output);
    assert_pinned_modules_present(&sys_external(&entry));

    // The key is the user-facing version (`4.91`), not the zero-padded
    // field version.txt carries (`04.9100`) nor the PUP header's opaque
    // image_version -- and the file it was read from is in the committed
    // tree, so a reader can re-derive the key from the entry alone.
    assert!(
        entry
            .join(FLASH_MOUNT)
            .join("vsh/etc/version.txt")
            .is_file(),
        "the tree the version key was read from must be the committed one",
    );
    let (major, minor) = version
        .split_once('.')
        .unwrap_or_else(|| panic!("version key {version:?} is not <major>.<minor>"));
    for part in [major, minor] {
        assert!(
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
            "version key {version:?} has a non-numeric part",
        );
    }

    // The manifest describes the firmware image, so it sits in the
    // mount it covers rather than at the entry root, and it declares
    // the same version the directory is named after.
    let manifest_path = entry.join(FLASH_MOUNT).join("firmware.toml");
    let manifest = cellgov_install::manifest::parse_manifest(
        &std::fs::read_to_string(&manifest_path).unwrap(),
    )
    .expect("the install wrote a manifest this build reads");
    assert_eq!(manifest.firmware.version, version);

    // Records are the store's index, so an installed version is one
    // that has one.
    let layout = StoreLayout::new(&*output);
    let record_path = layout.record_path(&Artifact::Firmware {
        version: VersionKey::new(&version).unwrap(),
    });
    let record = InstallRecord::parse(&std::fs::read_to_string(&record_path).unwrap())
        .expect("the install wrote a record this build reads");
    assert_eq!(record.artifact.kind, ArtifactKind::Firmware);
    assert_eq!(record.artifact.version, version);
    // The record is what a reader resolves the tree through, so a
    // store_path naming anything but the committed entry is an index
    // pointing away from its own install.
    assert_eq!(
        layout.resolve_store_path(&record.artifact.store_path),
        entry,
        "the record must resolve back to the entry it describes",
    );
    assert_eq!(record.source.kind, "pup");
    assert_eq!(record.source.sha256, manifest.firmware.pup_sha256);
    assert!(record.title.is_none(), "a firmware record names no title");
}

#[test]
fn install_stores_the_kernel_the_core_os_package_carries_beside_the_tree() {
    use cellgov_ps3_abi::format::core_os::LV2_KERNEL_SELF;

    let pup = locate_pup();
    let output = scratch::scratch_labeled("fw_kernel");
    let outcome =
        run_install(&pup, &output, false).unwrap_or_else(|e| panic!("install failed: {e}"));

    let kernel = outcome.core_os.kernel.as_ref().unwrap_or_else(|| {
        panic!(
            "the retail PUP carries a CoreOS package with a kernel, and the install \
             recorded none: {:?}",
            outcome.core_os.omission
        )
    });
    let stored = outcome.entry_dir.join("core_os").join(LV2_KERNEL_SELF);
    assert_eq!(kernel.path, "core_os/lv2_kernel.self");
    let bytes = std::fs::read(&stored).unwrap_or_else(|e| panic!("read {}: {e}", stored.display()));
    assert!(
        bytes.starts_with(&cellgov_ps3_abi::format::sce::SCE_MAGIC),
        "the kernel is kept SCE-wrapped as shipped"
    );
    assert_eq!(
        kernel.stored_sha256,
        cellgov_install::manifest::Sha256(cellgov_install::manifest::sha256_of(&bytes)),
        "the recorded digest is over the bytes as stored"
    );
    // The unpack keeps the kernel alone; the table names every entry,
    // kernel included.
    assert_eq!(
        std::fs::read_dir(outcome.entry_dir.join("core_os"))
            .unwrap()
            .count(),
        1
    );
    let names: Vec<&str> = outcome
        .core_os
        .files
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&LV2_KERNEL_SELF), "{names:?}");
    assert!(
        names.len() > 1,
        "the table names more than the kernel: {names:?}"
    );
    assert!(
        !outcome.entry_dir.join(FLASH_MOUNT).join("core_os").exists(),
        "the kernel sits beside the mount, not inside it"
    );

    // The record carries the same block, and a verify hashes the file.
    let layout = StoreLayout::new(&*output);
    let record_path = layout.record_path(&Artifact::Firmware {
        version: VersionKey::new(&outcome.version).unwrap(),
    });
    let record = InstallRecord::parse(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
    let recorded = record
        .core_os
        .and_then(|block| block.kernel)
        .expect("the record names the kernel");
    assert_eq!(&recorded, kernel);
    assert!(
        cellgov_install::firmware_verify::verify_stored_kernel(&outcome.entry_dir, &recorded)
            .expect("readable")
            .is_none(),
        "a fresh install verifies clean"
    );
}

#[test]
fn reinstalling_the_same_pup_is_refused_without_force_and_leaves_no_residue() {
    let pup = locate_pup();
    let output = scratch::scratch_labeled("fw_refuse");
    assert_succeeded(run_install(&pup, &output, false), "first install");
    let (_, entry) = sole_entry(&output);
    let before = std::fs::read_dir(entry.join(FLASH_MOUNT)).unwrap().count();

    let refusal = run_install(&pup, &output, false)
        .expect_err("the second install must refuse an installed version");
    // The same PUP twice has its own refusal, distinct from the one for
    // a second PUP claiming the same version; a bare "already installed"
    // would pass on either.
    let text = refusal.to_string();
    assert!(
        text.contains("is already installed, from this same PUP"),
        "expected the same-PUP duplicate refusal, got: {text}",
    );

    // The version cannot be read before the tree is extracted, so a
    // refusal always has a full staging tree to discard.
    assert!(
        !StoreLayout::new(&*output).firmware_staging_dir().exists(),
        "a refused install must discard its staging tree"
    );
    assert_eq!(
        std::fs::read_dir(entry.join(FLASH_MOUNT)).unwrap().count(),
        before,
        "the installed entry is untouched by the refused install"
    );
}

#[test]
fn a_populated_vfs_root_does_not_block_a_firmware_install() {
    let pup = locate_pup();
    let output = scratch::scratch_labeled("fw_sibling");
    std::fs::create_dir_all(output.join("dev_hdd0/game/NPUA80001")).unwrap();
    std::fs::write(output.join("dev_hdd0/game/NPUA80001/x.bin"), b"game").unwrap();

    assert_succeeded(run_install(&pup, &output, false), "install");
    assert!(
        output.join("dev_hdd0/game/NPUA80001/x.bin").is_file(),
        "the firmware install must not disturb a sibling mount"
    );
    // A successful install that wrote nothing would satisfy the
    // assertion above, so hold this run to the same output floor as the
    // empty-VFS one.
    let (_, entry) = sole_entry(&output);
    assert_pinned_modules_present(&sys_external(&entry));
}

#[test]
fn force_replaces_an_installed_version_whole() {
    let pup = locate_pup();
    let output = scratch::scratch_labeled("fw_force");
    assert_succeeded(run_install(&pup, &output, false), "first install");
    let (_, entry) = sole_entry(&output);
    let stale = entry.join(FLASH_MOUNT).join("stale.bin");
    std::fs::write(&stale, b"left over from the previous install").unwrap();

    assert_succeeded(run_install(&pup, &output, true), "install --force");
    assert!(
        !stale.exists(),
        "--force replaces the entry whole rather than merging into it"
    );
    // An empty entry would satisfy the check above, so --force is held
    // to the same output floor as a clean install.
    assert_pinned_modules_present(&sys_external(&entry));
    // Replacement, not a second entry beside the first, and no staging
    // tree left under the firmware root.
    assert_eq!(sole_entry(&output).1, entry);
}
