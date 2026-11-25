//! A vector left or right load or store that takes the alignment
//! interrupt in caching-inhibited storage keeps the R write and the group
//! read of its table search at any alignment, and a store keeps its C
//! write too.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_C_BYTE, PTE_I, PTE_R, PTE_R_BYTE};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, MSR_VXU, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The segment that the SLB maps, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0, which holds the code.
const CODE_RA: u64 = 0x3000;
/// Page 4, which holds each operand, and its real address.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + 0x4000;
const PAGE_RA: u64 = 0x6000;
/// Doubleword 1 of page 4's entry: caching-inhibited, read/write, with
/// R = C = 0.
const PAGE_D1: u64 = PAGE_RA | PTE_I | 0b10;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

/// An X-form vector access `vt,0,6` with extended opcode `xo`.
fn vx(xo: u32) -> u32 {
    (31 << 26) | (4 << 21) | (6 << 11) | (xo << 1)
}

const LVLX: u32 = 519;
const LVLXL: u32 = 775;
const LVRX: u32 = 551;
const LVRXL: u32 = 807;
const STVLX: u32 = 647;
const STVLXL: u32 = 903;
const STVRX: u32 = 679;
const STVRXL: u32 = 935;

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// The real address of the entry for page `index` of the segment: slot 0
/// of its primary group, whose hash is VSID XOR the page index.
fn entry(index: u64) -> u64 {
    HTAB + ((VSID ^ index) & 0x7FF) * GROUP
}

/// Writes the entry for page `index` with doubleword 1 `d1`.
fn map_page(mem: &mut GuestMemory, index: u64, d1: u64) {
    let d0 = (VSID << 12) | 1;
    put(mem, entry(index), &d0.to_be_bytes());
    put(mem, entry(index) + 8, &d1.to_be_bytes());
}

/// 512 KB of memory with `insn` at the start of page 0 and page 4 mapped
/// with [`PAGE_D1`].
fn memory(insn: u32) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, CODE_RA | 0b10);
    map_page(&mut mem, PAGE, PAGE_D1);
    put(&mut mem, CODE_RA, &insn.to_be_bytes());
    mem
}

/// One batch of one instruction, by a unit at the segment's first word
/// with relocation, MSR\[VXU\] and MSR\[ME\] on, and r6 at `ea`.
fn run_at(insn: u32, ea: u64) -> (ExecutionStepResult, Vec<Effect>) {
    let mem = memory(insn);
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR | MSR_ME | MSR_VXU);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
    unit.state_mut().set_gpr(6, ea);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ExecutionContext::new(&mem), &mut effects);
    (r, effects)
}

/// The bytes of the one-byte write intents to byte `byte` of page 4's
/// entry, in order.
fn entry_byte_writes(effects: &[Effect], byte: usize) -> Vec<u8> {
    let at = entry(PAGE) + byte as u64;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == at => {
                Some(bytes.bytes().to_vec())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Whether `effects` hold a read intent of page 4's whole group.
fn reads_group(effects: &[Effect]) -> bool {
    effects.iter().any(|e| {
        matches!(e, Effect::SharedReadIntent { range, .. }
            if range.start().raw() == entry(PAGE) && range.length() == GROUP)
    })
}

/// Byte 14 of page 4's entry after a search writes R.
///
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
fn r_byte() -> u8 {
    (PAGE_D1 | PTE_R).to_be_bytes()[PTE_R_BYTE - 8]
}

/// Byte 15 of page 4's entry after a search writes C.
///
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
fn c_byte() -> u8 {
    (PAGE_D1 | PTE_C).to_be_bytes()[PTE_C_BYTE - 8]
}

/// Asserts that a store at `ea` takes the alignment interrupt and keeps
/// the R and C writes and the group read of its search.
fn assert_store_keeps_search(xo: u32, ea: u64) {
    let (r, effects) = run_at(vx(xo), ea);
    let case = format!("xo {xo} ea {ea:#x}");
    assert_eq!(
        r.local_diagnostics.interrupt.map(|i| i.vector),
        Some(irq::VECTOR_ALIGNMENT),
        "{case}"
    );
    assert_eq!(
        entry_byte_writes(&effects, PTE_R_BYTE),
        vec![r_byte()],
        "{case}"
    );
    assert_eq!(
        entry_byte_writes(&effects, PTE_C_BYTE),
        vec![c_byte()],
        "{case}"
    );
    assert!(reads_group(&effects), "{case}");
}

/// [CBE-Handbook p:290 s:9.10] Table 9-39: a VSU store left or right that references caching-inhibited storage takes the alignment interrupt.
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup sets C = 0 to 1 unless the access takes a storage exception.
#[test]
fn a_stvrx_with_a_nonzero_byte_count_keeps_its_r_and_c_writes_in_inhibited_storage() {
    // EA 4 bytes into the quadword gives 4 bytes at page 4's first byte.
    for xo in [STVRX, STVRXL] {
        assert_store_keeps_search(xo, PAGE_EA + 4);
    }
}

/// [CBE-Handbook p:290 s:9.10] Table 9-39: a VSU store left or right that references caching-inhibited storage takes the alignment interrupt.
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup sets C = 0 to 1 unless the access takes a storage exception.
#[test]
fn a_stvlx_off_a_quadword_boundary_keeps_its_r_and_c_writes_in_inhibited_storage() {
    for xo in [STVLX, STVLXL] {
        assert_store_keeps_search(xo, PAGE_EA + 4);
    }
}

/// [CBE-Handbook p:290 s:9.10] Table 9-39: a VSU load left or right that references caching-inhibited storage takes the alignment interrupt.
/// [CBE-Handbook p:90 s:4.2.6.8] A load's table lookup is nonspeculative, so a lookup that matches an entry with R = 0 sets R.
/// [CBE-Handbook p:90 s:4.2.6.8] Only a store's lookup sets C.
#[test]
fn a_vector_left_or_right_load_keeps_its_r_write_and_group_read_in_inhibited_storage() {
    // (extended opcode, EA). An lvrx at a quadword boundary reads no
    // storage, so each lvrx EA is 4 bytes into the quadword.
    let cases = [
        (LVLX, PAGE_EA),
        (LVLX, PAGE_EA + 4),
        (LVLXL, PAGE_EA),
        (LVLXL, PAGE_EA + 4),
        (LVRX, PAGE_EA + 4),
        (LVRXL, PAGE_EA + 4),
    ];
    for (xo, ea) in cases {
        let (r, effects) = run_at(vx(xo), ea);
        let case = format!("xo {xo} ea {ea:#x}");
        assert_eq!(
            r.local_diagnostics.interrupt.map(|i| i.vector),
            Some(irq::VECTOR_ALIGNMENT),
            "{case}"
        );
        assert_eq!(
            entry_byte_writes(&effects, PTE_R_BYTE),
            vec![r_byte()],
            "{case}"
        );
        assert_eq!(
            entry_byte_writes(&effects, PTE_C_BYTE),
            Vec::<u8>::new(),
            "{case}"
        );
        assert!(reads_group(&effects), "{case}");
    }
}
