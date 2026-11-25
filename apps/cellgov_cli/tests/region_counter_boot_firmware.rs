//! Checks which hypercalls the stored 4.93 kernel makes, from its entry,
//! before its first `allocate_memory`.
//!
//! None of these calls may create a region, so the region counter holds
//! its value at the partition's start when the first allocation draws
//! from it.
//!
//! Requires:
//! - the `installed-firmware-tests` feature;
//! - an installed 4.93 entry that stores a kernel;
//! - the operator's LV2 keys.

use std::collections::BTreeSet;
use std::path::PathBuf;

use cellgov_boot::kernel::{run_kernel_from_entry, KernelRun};
use cellgov_install::keys::KeyVault;
use cellgov_install::self_image::{into_plaintext_elf, KeyPolicy};
use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, GET_LOGICAL_PARTITION_ID, LOGICAL_CONSOLE_CONSTRUCT, READ_REPOSITORY_NODE,
};

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

const FW: &str = installed_firmware::TEST_FIRMWARE_VERSION;

/// The largest step cap the search tries.
const MAX_STEPS: usize = 1_000_000;

/// The stored kernel of the installed [`FW`] entry.
fn stored_kernel() -> PathBuf {
    let store = installed_firmware::workspace_root().join(DEFAULT_VFS_ROOT);
    let layout = StoreLayout::new(&store);
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records)
        .unwrap_or_else(|e| panic!("read firmware install records {}: {e}", records.display()));
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
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let record =
            InstallRecord::parse(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        if record.artifact.version != FW {
            continue;
        }
        let kernel = record
            .core_os
            .as_ref()
            .and_then(|c| c.kernel.as_ref())
            .unwrap_or_else(|| panic!("firmware {FW}: the installed entry stores no kernel"));
        return kernel.path_in(&layout.resolve_store_path(&record.artifact.store_path));
    }
    panic!("firmware {FW}: no installed entry; install that firmware");
}

/// The hypercall numbers of `run` over both hardware threads.
fn numbers(run: &KernelRun) -> BTreeSet<u64> {
    run.hypercall_sites
        .iter()
        .flatten()
        .map(|site| site.number)
        .collect()
}

/// The run of `elf` with the smallest step cap whose run made an
/// `allocate_memory`. A run's sites grow with its cap, so that run holds
/// every hypercall up to and including the first allocation.
fn run_to_first_allocation(elf: &[u8]) -> KernelRun {
    let run = |cap: usize| {
        run_kernel_from_entry(elf, FW, cap)
            .unwrap_or_else(|e| panic!("firmware {FW}: run from entry: {e}"))
    };
    let allocates = |run: &KernelRun| numbers(run).contains(&ALLOCATE_MEMORY);
    let mut high = 1;
    let mut found = loop {
        let r = run(high);
        if allocates(&r) {
            break r;
        }
        assert!(
            high < MAX_STEPS,
            "firmware {FW}: no allocate_memory in {high} steps; the run stopped at {:?}",
            r.stop
        );
        high = (high * 2).min(MAX_STEPS);
    };
    let mut low = high / 2;
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        let r = run(mid);
        if allocates(&r) {
            high = mid;
            found = r;
        } else {
            low = mid;
        }
    }
    found
}

#[test]
fn the_kernel_makes_no_region_creating_call_before_its_first_allocation() {
    let store = installed_firmware::workspace_root().join(DEFAULT_VFS_ROOT);
    let vault =
        KeyVault::load_for_vfs(&store).unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));
    let path = stored_kernel();
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let elf = into_plaintext_elf(raw, &vault, KeyPolicy::AppOnly)
        .unwrap_or_else(|e| panic!("firmware {FW}: decrypt {}: {e}", path.display()));
    let run = run_to_first_allocation(&elf);
    assert_eq!(
        numbers(&run),
        BTreeSet::from([
            ALLOCATE_MEMORY,
            GET_LOGICAL_PARTITION_ID,
            READ_REPOSITORY_NODE,
            LOGICAL_CONSOLE_CONSTRUCT,
        ]),
        "firmware {FW}: the hypercalls up to the first allocation, in {} steps",
        run.steps
    );
}
