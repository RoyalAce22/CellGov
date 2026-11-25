use super::*;

const PUP_A: &str = "pup-a";
const KERNEL_A: &str = "aa";

fn kernels() -> Vec<KernelRow> {
    [(PUP_A, KERNEL_A), ("pup-b", "bb")]
        .into_iter()
        .map(|(pup_sha256, kernel_elf_sha256)| KernelRow {
            pup_sha256: pup_sha256.to_string(),
            kernel_elf_sha256: kernel_elf_sha256.to_string(),
            table_base: 0x1000,
            entry_width: 8,
            entry_format: "opd_be_u32_fn_u32_toc".to_string(),
            entry_count: 1,
            discovery_method: "toc_reference".to_string(),
            confidence: "high".to_string(),
            census_sha256: "cc".to_string(),
            subentry_sha256: "dd".to_string(),
            gate_sha256: "ee".to_string(),
        })
        .collect()
}

#[test]
fn an_elf_with_its_pups_recorded_kernel_digest_is_accepted() {
    assert_eq!(check_kernel_elf(&kernels(), PUP_A, KERNEL_A), Ok(()));
}

#[test]
fn an_elf_with_another_pups_kernel_digest_is_refused_as_a_mismatch() {
    let error = check_kernel_elf(&kernels(), PUP_A, "bb").unwrap_err();
    assert_eq!(
        error,
        KernelElfError::KernelMismatch {
            pup_sha256: PUP_A.to_string(),
            expected: KERNEL_A.to_string(),
            found: "bb".to_string(),
        }
    );
    assert_eq!(
        error.to_string(),
        "kernel ELF SHA-256 bb is not the kernel aa that kernel.tsv names for PUP pup-a"
    );
}

#[test]
fn a_pup_with_no_kernel_row_is_refused() {
    let error = check_kernel_elf(&kernels(), "pup-c", KERNEL_A).unwrap_err();
    assert_eq!(
        error,
        KernelElfError::NoKernelRow {
            pup_sha256: "pup-c".to_string(),
        }
    );
    assert_eq!(
        error.to_string(),
        "PUP pup-c has no kernel.tsv row; run lv2-census for it first"
    );
}
