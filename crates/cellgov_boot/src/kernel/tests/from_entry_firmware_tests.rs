use cellgov_lv2::hypervisor::HypercallRefusal;
use cellgov_ps3_abi::lv1::repository::{field, first_field, lookup, RepositoryKey, SecondValue};

use super::{kernel_elf, run_kernel_from_entry, KernelRunStop};
use super::{B_SELF, LIS_R4_BI, LIS_R5_PU, LI_R11_91};
use super::{LI_R3_2, SC_1, SLDI_R5_32};

/// `li r6, 1`: PU id 1.
const LI_R6_1: u32 = 0x38c0_0001;
/// `lis r7, 0x726d`, `ori r7, r7, 0x5f73`, `sldi r7, r7, 32`,
/// `oris r7, r7, 0x697a`, `ori r7, r7, 0x6500`: r7 = `rm_size`.
const R7_RM_SIZE: [u32; 5] = [
    0x3ce0_726d,
    0x60e7_5f73,
    0x78e7_07c6,
    0x64e7_697a,
    0x60e7_6500,
];

/// One read of `bi.pu.1.rm_size`, then a spin.
fn rm_size_kernel() -> Vec<u8> {
    let mut code = vec![LI_R3_2, LIS_R4_BI, LIS_R5_PU, SLDI_R5_32, LI_R6_1];
    code.extend_from_slice(&R7_RM_SIZE);
    code.extend_from_slice(&[LI_R11_91, SC_1, B_SELF]);
    kernel_elf(&code)
}

fn rm_size_key() -> RepositoryKey {
    RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: field(b"pu", 0),
        n3: 1,
        n4: field(b"rm_size", 0),
    }
}

/// The values the run's first repository read answered at `fw`.
fn answered_at(fw: &str) -> Option<(u64, u64)> {
    let run = run_kernel_from_entry(&rm_size_kernel(), fw, 50).unwrap();
    let read = run
        .repository_reads
        .first()
        .expect("the kernel reads rm_size");
    assert_eq!(read.key, rm_size_key());
    read.values
}

/// At 1.50 CellGov serves the psdevwiki 3.41 dump, which gives both
/// values. At 4.93 the console capture does not observe the second
/// value, and this kernel's read site is not one that discards it.
#[test]
fn a_kernel_run_takes_its_repository_answers_from_the_firmware_it_names() {
    let key = rm_size_key();
    let dump = lookup(&key, "1.50").expect("rm_size at 1.50");
    assert_eq!(dump.v2, SecondValue::Given(0));
    assert_eq!(answered_at("1.50"), Some((dump.v1, 0)));
    assert_eq!(answered_at("4.93"), None);
    let run = run_kernel_from_entry(&rm_size_kernel(), "4.93", 50).unwrap();
    assert!(
        matches!(
            run.stop,
            KernelRunStop::Refused(HypercallRefusal::RepositorySecondValueUnobserved { key: k, .. })
                if k == key
        ),
        "the run stopped at {:?}",
        run.stop
    );
}
