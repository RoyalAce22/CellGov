use cellgov_event::UnitId;
use cellgov_lv2::hypervisor::{HypercallRefusal, RepositoryRead};
use cellgov_ps3_abi::format::core_os::LV2_KERNEL_LINK_BASE;
use cellgov_ps3_abi::hw::ppu::CELL_BP_VR;
use cellgov_ps3_abi::lv1::repository::{field, first_field, RepositoryKey};

use super::map::KERNEL_SPAN_LIMIT;
use super::{run_kernel_from_entry, KernelRunError, KernelRunStop};

#[path = "from_entry_firmware_tests.rs"]
mod firmware;

#[path = "from_entry_reader_tests.rs"]
mod reader;

#[path = "from_entry_page_table_tests.rs"]
mod page_table;

/// A firmware the psdevwiki dump's repository values are served to.
const FW: &str = "1.50";

const BASE: u64 = LV2_KERNEL_LINK_BASE;
const ENTRY: u64 = BASE + 0x100;

/// A big-endian PPC64 executable with one read-execute PT_LOAD at the
/// link base that holds `code` from offset 0x100, and its entry there.
fn kernel_elf(code: &[u32]) -> Vec<u8> {
    kernel_elf_at(BASE, 0, code)
}

/// [`kernel_elf`] with its PT_LOAD at `vaddr`, its entry at `vaddr +
/// 0x100`, and `bss` zero-fill bytes past its file bytes.
fn kernel_elf_at(vaddr: u64, bss: u64, code: &[u32]) -> Vec<u8> {
    let mut text = vec![0u8; 0x100];
    for word in code {
        text.extend_from_slice(&word.to_be_bytes());
    }
    let phoff = 64usize;
    let mut buf = vec![0u8; phoff + 56];
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2;
    buf[5] = 2;
    buf[6] = 1;
    buf[16..18].copy_from_slice(&2u16.to_be_bytes());
    buf[18..20].copy_from_slice(&21u16.to_be_bytes());
    buf[24..32].copy_from_slice(&(vaddr + 0x100).to_be_bytes());
    buf[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
    buf[54..56].copy_from_slice(&56u16.to_be_bytes());
    buf[56..58].copy_from_slice(&1u16.to_be_bytes());
    let offset = buf.len() as u64;
    let len = text.len() as u64;
    buf.extend_from_slice(&text);
    let ph = phoff;
    buf[ph..ph + 4].copy_from_slice(&1u32.to_be_bytes());
    buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_be_bytes());
    buf[ph + 8..ph + 16].copy_from_slice(&offset.to_be_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&vaddr.to_be_bytes());
    buf[ph + 24..ph + 32].copy_from_slice(&vaddr.to_be_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&len.to_be_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&(len + bss).to_be_bytes());
    buf
}

const LI_R3_2: u32 = 0x3860_0002;
const LIS_R4_BI: u32 = 0x3c80_6269;
const LIS_R5_PU: u32 = 0x3ca0_7075;
const ORI_R5_N: u32 = 0x60a5_6e00;
const SLDI_R5_32: u32 = 0x78a5_07c6;
const LI_R5_0: u32 = 0x38a0_0000;
const LI_R11_91: u32 = 0x3960_005b;
const SC_1: u32 = 0x4400_0022;
const B_SELF: u32 = 0x4800_0000;

/// A read of `bi.pun`, which the repository dump
/// [PS3DEVWIKI-REPO s:3.41_GameOS] answers with 1, then a read of the
/// bare `bi`, which no source gives.
const TWO_READS: &[u32] = &[
    LI_R3_2, LIS_R4_BI, LIS_R5_PU, ORI_R5_N, SLDI_R5_32, LI_R11_91, SC_1, // bi.pun
    LI_R3_2, LIS_R4_BI, LI_R5_0, LI_R11_91, SC_1, // bi
    B_SELF,
];

#[test]
fn a_read_of_a_node_with_no_row_stops_the_kernel_at_a_refusal_that_names_the_key() {
    let run = run_kernel_from_entry(&kernel_elf(TWO_READS), FW, 10_000).unwrap();
    let bare_bi = RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: 0,
        n3: 0,
        n4: 0,
    };
    let second_sc = ENTRY + 4 * 11;
    let KernelRunStop::Refused(HypercallRefusal::RepositoryNode { key, pc, .. }) = run.stop else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!((key, pc), (bare_bi, second_sc));
    assert_eq!(
        run.repository_reads,
        vec![
            RepositoryRead {
                key: RepositoryKey {
                    n2: field(b"pun", 0),
                    ..bare_bi
                },
                values: Some((1, 0)),
                unit: UnitId::new(1),
                pc: ENTRY + 4 * 6,
            },
            RepositoryRead {
                key: bare_bi,
                values: None,
                unit: UnitId::new(1),
                pc: second_sc,
            },
        ]
    );
}

/// `mfspr r0, 287` reads the PVR, which the PPU does not model in
/// supervisor state: the unit faults instead of taking an interrupt.
const MFSPR_R0_PVR: u32 = 0x7c1f_42a6;

/// `mfspr r0, 1022` reads BP_VR [CBEA p:281 s:23.1].
const MFSPR_R0_BP_VR: u32 = 0x7c1e_faa6;

#[test]
fn a_kernel_fault_stops_the_run_at_the_faulting_pc_and_the_sibling_runs_no_further() {
    let run = run_kernel_from_entry(&kernel_elf(&[LI_R3_2, MFSPR_R0_PVR]), FW, 10_000).unwrap();
    let KernelRunStop::Fault { pc, .. } = run.stop else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!(pc, Some(ENTRY + 4));
    assert!(run.steps < 10_000, "{} steps", run.steps);
}

/// `cmpdi cr0, r0, CELL_BP_VR`.
const CMPDI_R0_BP_VR: u32 = 0x2c20_0000 | CELL_BP_VR;
const _: () = assert!(
    CELL_BP_VR < 0x8000,
    "the value fits SI as a positive immediate"
);
/// `bne +8`.
const BNE_8: u32 = 0x4082_0008;

#[test]
fn a_kernel_reads_bp_vr_without_an_interrupt_and_gets_the_modelled_value() {
    // A wrong value branches to the `b .` and runs to the step cap. An
    // interrupt leaves for the program vector. Only the right value
    // reaches the unmodelled PVR read.
    let code = &[MFSPR_R0_BP_VR, CMPDI_R0_BP_VR, BNE_8, MFSPR_R0_PVR, B_SELF];
    let run = run_kernel_from_entry(&kernel_elf(code), FW, 10_000).unwrap();
    let KernelRunStop::Fault { pc, .. } = run.stop else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!(pc, Some(ENTRY + 12));
}

#[test]
fn a_kernel_that_never_calls_out_stops_at_the_step_cap() {
    let run = run_kernel_from_entry(&kernel_elf(&[B_SELF]), FW, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    assert_eq!(run.steps, 50);
    assert!(run.repository_reads.is_empty());
}

const LI_R4_0: u32 = 0x3880_0000;
const LIS_R3_0400: u32 = 0x3c60_0400;
/// `stw r4, -4(r3)`.
const STW_R4_M4_R3: u32 = 0x9083_fffc;

#[test]
fn a_kernel_store_over_the_sibling_word_stops_the_run_at_a_sibling_fault() {
    // Writes 0 over real 0x3fffffc, the sibling's `b .`, then spins.
    let code = &[LI_R4_0, LIS_R3_0400, STW_R4_M4_R3, B_SELF];
    let run = run_kernel_from_entry(&kernel_elf(code), FW, 10_000).unwrap();
    assert!(
        matches!(run.stop, KernelRunStop::SiblingFault { .. }),
        "the run stopped at {:?}",
        run.stop
    );
    assert!(run.steps < 10_000, "{} steps", run.steps);
}

#[test]
fn a_segment_whose_zero_fill_reaches_the_sibling_word_is_refused_and_one_word_lower_runs() {
    let real = KERNEL_SPAN_LIMIT - 0x1000;
    let file_len = 0x104;
    let reaching = kernel_elf_at(BASE + real, 0x1000 - file_len, &[B_SELF]);
    let error = run_kernel_from_entry(&reaching, FW, 4).unwrap_err();
    assert!(
        matches!(error, KernelRunError::OutsideRegion { at, len } if (at, len) == (real, 0x1000)),
        "{error}"
    );

    let below = kernel_elf_at(BASE + real, 0x1000 - file_len - 4, &[B_SELF]);
    let run = run_kernel_from_entry(&below, FW, 4).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
}
