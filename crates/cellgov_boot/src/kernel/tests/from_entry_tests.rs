use cellgov_event::UnitId;
use cellgov_lv1::{HypercallRefusal, RepositoryRead};
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

#[path = "from_entry_allocated_tests.rs"]
mod allocated;

#[path = "from_entry_position_tests.rs"]
mod position;

#[path = "from_entry_threads_tests.rs"]
mod threads;

/// A firmware for which CellGov serves the repository values of the
/// psdevwiki dump.
const FW: &str = "1.50";

const BASE: u64 = LV2_KERNEL_LINK_BASE;
/// The entry of thread 0, one word past the `b .` that hardware thread 1
/// runs from its reset vector at real 0x100.
const ENTRY: u64 = BASE + 0x104;

/// A big-endian PPC64 executable with one read-execute PT_LOAD at the
/// link base. The PT_LOAD holds a `b .` at offset 0x100 and `code` from
/// offset 0x104, and the entry is at offset 0x104.
fn kernel_elf(code: &[u32]) -> Vec<u8> {
    kernel_elf_at(BASE, 0, code)
}

/// [`kernel_elf`] with its PT_LOAD at `vaddr`, its entry at `vaddr +
/// 0x104`, and `bss` zero-fill bytes past its file bytes.
fn kernel_elf_at(vaddr: u64, bss: u64, code: &[u32]) -> Vec<u8> {
    let mut words = vec![B_SELF];
    words.extend_from_slice(code);
    elf_image(vaddr, bss, &words, 0x104)
}

/// A big-endian PPC64 executable with its entry at `vaddr +
/// entry_offset` and one read-execute PT_LOAD at `vaddr`. The PT_LOAD
/// holds `words` from offset 0x100, and `bss` zero-fill bytes past them.
fn elf_image(vaddr: u64, bss: u64, words: &[u32], entry_offset: u64) -> Vec<u8> {
    let mut text = vec![0u8; 0x100];
    for word in words {
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
    buf[24..32].copy_from_slice(&(vaddr + entry_offset).to_be_bytes());
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
                unit: UnitId::new(0),
                pc: ENTRY + 4 * 6,
            },
            RepositoryRead {
                key: bare_bi,
                values: None,
                unit: UnitId::new(0),
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
fn a_thread_0_fault_stops_the_run_at_the_faulting_pc_and_names_thread_0() {
    let run = run_kernel_from_entry(&kernel_elf(&[LI_R3_2, MFSPR_R0_PVR]), FW, 10_000).unwrap();
    let KernelRunStop::Fault { thread, pc, .. } = run.stop else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!((thread, pc), (0, Some(ENTRY + 4)));
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
    // interrupt enters the program vector. Only the right value
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

#[test]
fn a_segment_whose_zero_fill_passes_the_region_end_is_refused_and_one_ending_there_maps() {
    let real = KERNEL_SPAN_LIMIT - 0x1000;
    // The file holds 0x100 bytes of padding, thread 1's `b .` and one
    // `b .` of code.
    let file_len = 0x108;
    let past = kernel_elf_at(BASE + real, 0x1000 - file_len + 4, &[B_SELF]);
    let error = run_kernel_from_entry(&past, FW, 4).unwrap_err();
    assert!(
        matches!(error, KernelRunError::OutsideRegion { at, len } if (at, len) == (real, 0x1004)),
        "{error}"
    );

    // Thread 0 takes the first step. Its pc stays on the mapped `b .` at
    // its entry only when the segment's bytes are in the region; an
    // unmapped word faults and adds no recent pc.
    let ending = kernel_elf_at(BASE + real, 0x1000 - file_len, &[B_SELF]);
    let run = run_kernel_from_entry(&ending, FW, 4).unwrap();
    let entry = BASE + real + 0x104;
    let thread0 = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(thread0.pc, entry);
    assert_eq!(
        thread0.recent_pcs.first(),
        Some(&entry),
        "the run stopped at {:?}",
        run.stop
    );
}
