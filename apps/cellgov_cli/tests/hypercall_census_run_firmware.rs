//! Checks the hypercall census of each logged firmware's stored kernel
//! against the hypercalls that the kernel makes when it runs from its
//! entry.
//!
//! A run observes only the paths that it takes. So it can disprove the
//! census at a site that it reaches, and it proves nothing about a site
//! that it does not reach. The test checks each observed site:
//! - a site that the census does not find is a miss;
//! - a site that the census resolves to another number is a wrong
//!   resolve;
//! - a site that the census leaves unresolved is printed with the
//!   numbers that the run gave there, and is never a resolve.
//!
//! Requires:
//! - the `installed-firmware-tests` feature;
//! - an installed entry with a stored kernel for each logged firmware;
//! - the operator's LV2 keys.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries each kernel's observed sites"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_boot::kernel::{run_kernel_from_entry, KernelRun};
use cellgov_install::keys::KeyVault;
use cellgov_install::self_image::{into_plaintext_elf, KeyPolicy};
use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_lv1_archive::REPOSITORY_LOGGED_FIRMWARE;
use cellgov_ppu::caller_census::scan_hypercalls;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

/// The step cap of each run: the cap that `cellgov dev repository-census`
/// takes by default for the boots that it logs.
const MAX_STEPS: usize = 1_000_000;

/// How an observed site disagrees with the census.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mismatch {
    /// The census finds no site at the pc.
    Miss,
    /// The census resolves the site to this number.
    WrongResolve(u16),
}

/// One observed site that disagrees with the census.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    fw: String,
    /// The real address of the `sc`.
    pc: u64,
    mismatch: Mismatch,
    /// The numbers that the run gave at the `sc`, in ascending order.
    numbers: Vec<u64>,
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The stored kernel of each installed firmware entry that stores one,
/// keyed by version.
fn stored_kernels(store: &Path) -> BTreeMap<String, PathBuf> {
    let layout = StoreLayout::new(store);
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
        kernels.insert(record.artifact.version.clone(), kernel.path_in(&entry_dir));
    }
    kernels
}

/// The real address of the effective address `ea`.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing ignores EA bit 1, and EA bit 0 picks the mechanism that forms the real address.
fn real(ea: u64) -> u64 {
    ea & !REAL_MODE_IGNORED_EA
}

/// The numbers that `run` gave at each `sc`, by the real address of the
/// `sc`, over both hardware threads.
fn observed_sites(run: &KernelRun) -> BTreeMap<u64, BTreeSet<u64>> {
    let mut sites: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for site in run.hypercall_sites.iter().flatten() {
        sites.entry(real(site.pc)).or_default().insert(site.number);
    }
    sites
}

/// The census of `elf`: the number of each site, or `None` for a site
/// that the census leaves unresolved, by the real address of the site.
///
/// # Panics
///
/// When the scan refuses the image, or two sites have one real address.
fn census_sites(fw: &str, elf: &[u8]) -> BTreeMap<u64, Option<u16>> {
    let sites = scan_hypercalls(elf).unwrap_or_else(|e| panic!("firmware {fw}: {e}"));
    let mut census = BTreeMap::new();
    for site in &sites {
        let previous = census.insert(real(site.address), site.ordinal);
        assert!(
            previous.is_none(),
            "firmware {fw}: two census sites have the real address 0x{:x}",
            real(site.address)
        );
    }
    census
}

/// What the check of one kernel found.
struct KernelCheck {
    findings: Vec<Finding>,
    /// The number of observed sites that the census resolves to the one
    /// number that the run gave there.
    matched: usize,
    /// Each observed site that the census leaves unresolved, with the
    /// numbers that the run gave there.
    unresolved: Vec<(u64, Vec<u64>)>,
}

fn check_kernel(
    fw: &str,
    observed: &BTreeMap<u64, BTreeSet<u64>>,
    census: &BTreeMap<u64, Option<u16>>,
) -> KernelCheck {
    let mut check = KernelCheck {
        findings: Vec::new(),
        matched: 0,
        unresolved: Vec::new(),
    };
    for (&pc, numbers) in observed {
        let numbers: Vec<u64> = numbers.iter().copied().collect();
        let mismatch = match census.get(&pc) {
            None => Mismatch::Miss,
            Some(None) => {
                check.unresolved.push((pc, numbers));
                continue;
            }
            Some(Some(resolved)) if numbers == [u64::from(*resolved)] => {
                check.matched += 1;
                continue;
            }
            Some(Some(resolved)) => Mismatch::WrongResolve(*resolved),
        };
        check.findings.push(Finding {
            fw: fw.to_string(),
            pc,
            mismatch,
            numbers,
        });
    }
    check
}

#[test]
fn each_hypercall_that_a_logged_kernel_makes_is_a_census_site_of_the_same_number() {
    let store = installed_firmware::workspace_root().join(DEFAULT_VFS_ROOT);
    let kernels = stored_kernels(&store);
    let vault =
        KeyVault::load_for_vfs(&store).unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));

    let mut findings = BTreeSet::new();
    assert_eq!(REPOSITORY_LOGGED_FIRMWARE.len(), 4);
    for fw in REPOSITORY_LOGGED_FIRMWARE {
        let path = kernels.get(*fw).unwrap_or_else(|| {
            panic!("firmware {fw}: no installed entry stores a kernel; install that firmware")
        });
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let elf = into_plaintext_elf(raw, &vault, KeyPolicy::AppOnly)
            .unwrap_or_else(|e| panic!("firmware {fw}: decrypt {}: {e}", path.display()));
        let run = run_kernel_from_entry(&elf, fw, MAX_STEPS)
            .unwrap_or_else(|e| panic!("firmware {fw}: run from entry: {e}"));
        let observed = observed_sites(&run);
        assert!(
            !observed.is_empty(),
            "firmware {fw}: the run made no hypercall"
        );
        let census = census_sites(fw, &elf);
        let check = check_kernel(fw, &observed, &census);
        eprintln!(
            "firmware {fw}: {} steps, stopped at {:?}; {} observed pairs at {} sites; \
             {} census sites; {} matched; {} findings; \
             {} observed sites the census leaves unresolved",
            run.steps,
            run.stop,
            run.hypercall_sites.iter().map(BTreeSet::len).sum::<usize>(),
            observed.len(),
            census.len(),
            check.matched,
            check.findings.len(),
            check.unresolved.len()
        );
        for finding in &check.findings {
            eprintln!(
                "  {:?} at real 0x{:x}: the run gave {:?}",
                finding.mismatch, finding.pc, finding.numbers
            );
        }
        for (pc, numbers) in &check.unresolved {
            eprintln!("  unresolved at real 0x{pc:x}: the run gave {numbers:?}");
        }
        // A run whose sites the census all leaves unresolved checks no number.
        assert!(
            check.matched > 0,
            "firmware {fw}: no observed site has the census number that the run gave"
        );
        findings.extend(check.findings);
    }

    assert!(
        findings.is_empty(),
        "the census disagrees with the kernel runs: {findings:#x?}"
    );
}
