//! A register word of an unmodelled span answers its reader's load while
//! the bits the reader keeps are known, takes a whole-word store that its
//! device accepts, and ends the batch after either access.

use crate::fault_codes::FAULT_UNMODELLED_STORAGE;
use crate::instruction::{FusedPair, PpuInstruction};
use crate::mmu::{Access, Translation};
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_C_BYTE};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The code page: page 0 of the segment, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// The page of the segment that maps the span: page 4.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + (PAGE << 12);
/// The unmodelled span: one 4 KB page past the 512 KB the memory holds.
const SPAN: u64 = 0x9_0000;
const SPAN_LEN: u64 = 0x1000;
/// PP = 2 with the caching-inhibited bit, as a device's MMIO page has.
const PTE_LOW: u64 = 0b10 | 0x20;
/// The offset of the register word in the span.
const OFFSET: u64 = 0x10;
/// The register word's value while it is known.
const WORD: u32 = 0x0000_0010;

const TRANSLATED: u64 = MSR_SF | MSR_IR | MSR_DR | MSR_ME;

/// `lwz r4,0(r5)`.
const LWZ: u32 = (32 << 26) | (4 << 21) | (5 << 16);
/// `stw r4,0(r5)`.
const STW: u32 = (36 << 26) | (4 << 21) | (5 << 16);
/// `stb r4,0(r5)`.
const STB: u32 = (38 << 26) | (4 << 21) | (5 << 16);
/// `li r6,1`.
const LI: u32 = (14 << 26) | (6 << 21) | 1;

/// The device's rule: a store with bit 0 set is refused, and any other
/// store becomes the known word.
fn refuse_bit_0(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    if store.value & 1 != 0 {
        return false;
    }
    words[index] = RegisterWord::known(store.value);
    true
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Writes the entry for page `index` of the segment to slot 0 of its
/// primary group.
fn map_page(mem: &mut GuestMemory, index: u64, ra: u64, low: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | low).to_be_bytes());
}

/// 512 KB of memory with `code` on the code page and [`SPAN`] declared
/// unmodelled behind page [`PAGE`]. The word at [`OFFSET`] is a register
/// word in the state `word`, whose reader is the segment's first word and
/// keeps the low 4 bits.
fn memory(code: &[u32], word: RegisterWord) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    mem.declare_unmodelled(SPAN, SPAN_LEN).unwrap();
    mem.declare_registers(&[(SPAN + OFFSET, word)], refuse_bit_0)
        .unwrap();
    mem.declare_register_reader(SPAN + OFFSET, SEGMENT, 0xF, None)
        .unwrap();
    map_page(&mut mem, 0, CODE_RA, 0b10);
    map_page(&mut mem, PAGE, SPAN, PTE_LOW);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// Runs a batch of `budget` instructions from the segment's first word,
/// with r4 at `r4`, r6 at 0x5A and r5 at the register word's EA.
fn run(
    mem: &GuestMemory,
    budget: u64,
    r4: u64,
) -> (ExecutionStepResult, PpuExecutionUnit, Vec<Effect>) {
    let mut sup = SupervisorState::new(TRANSLATED);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition =
        crate::mmu::Partition::new(HTAB, 1 << 62, 0, crate::mmu::Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
    unit.state_mut().set_gpr(4, r4);
    unit.state_mut().set_gpr(5, PAGE_EA + OFFSET);
    unit.state_mut().set_gpr(6, 0x5A);
    let mut effects = Vec::new();
    let ctx = ExecutionContext::new(mem);
    let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    (r, unit, effects)
}

/// The writes of `effects` into the span, each as its range's start and
/// its bytes. The page table's R and C writes are outside the span.
fn writes(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. }
                if (SPAN..SPAN + SPAN_LEN).contains(&range.start().raw()) =>
            {
                Some((range.start().raw(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .collect()
}

const STOP_AT_WORD: (Option<FaultKind>, Option<u64>, Option<u64>) = (
    Some(FaultKind::Guest(FAULT_UNMODELLED_STORAGE)),
    Some(PAGE_EA + OFFSET),
    Some(SPAN + OFFSET),
);

fn stop(r: &ExecutionStepResult) -> (Option<FaultKind>, Option<u64>, Option<u64>) {
    let d = &r.local_diagnostics;
    (r.fault, d.faulting_ea, d.faulting_ra)
}

#[test]
fn the_readers_load_gets_the_known_word_and_its_batch_ends_after_it() {
    let mem = memory(&[LWZ, LI], RegisterWord::known(WORD));
    let (r, unit, _) = run(&mem, 2, 0x5A);
    assert_eq!(r.fault, None);
    let gpr = unit.state().gpr.as_array();
    assert_eq!((gpr[4], gpr[6]), (u64::from(WORD), 0x5A));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn a_fused_pair_gets_no_register_word_for_its_own_address() {
    let mem = memory(&[], RegisterWord::known(WORD));
    let unit = PpuExecutionUnit::new(UnitId::new(0));
    let t = Translation {
        ea: PAGE_EA,
        ra: SPAN,
        len: SPAN_LEN,
        writable: true,
        inhibited: true,
        guarded: false,
        changed: true,
    };
    let at = (SPAN + OFFSET, Access::Load);
    let lwz = crate::decode::decode(LWZ).unwrap();
    let pair = PpuInstruction::Fused(FusedPair::LwzLwz {
        rt1: 4,
        ra1: 5,
        off1: 0,
        rt2: 6,
        ra2: 5,
        off2: 4,
    });
    let view = unit
        .register_view(&lwz, &mem, &[], &t, at, SEGMENT)
        .unwrap();
    assert_eq!((view.base, view.real), (PAGE_EA + OFFSET, SPAN + OFFSET));
    assert_eq!(view.bytes, WORD.to_be_bytes());
    assert!(unit
        .register_view(&pair, &mem, &[], &t, at, SEGMENT)
        .is_none());
}

#[test]
fn the_readers_load_stops_while_a_bit_it_keeps_is_unknown() {
    let word = RegisterWord {
        value: 0,
        known: 0x7,
    };
    let mem = memory(&[LWZ], word);
    let (r, unit, _) = run(&mem, 1, 0x5A);
    assert_eq!(stop(&r), STOP_AT_WORD);
    assert_eq!(unit.state().gpr.as_array()[4], 0x5A);
}

#[test]
fn an_accepted_store_publishes_one_word_write_and_ends_the_batch() {
    let mem = memory(&[STW, LI], RegisterWord::UNKNOWN);
    let (r, unit, effects) = run(&mem, 2, 0x5A);
    assert_eq!(r.fault, None);
    assert_eq!(writes(&effects), [(SPAN + OFFSET, vec![0, 0, 0, 0x5A])]);
    assert_eq!(unit.state().gpr.as_array()[6], 0x5A);
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn a_store_that_the_device_refuses_stops_at_the_word() {
    let mem = memory(&[STW], RegisterWord::UNKNOWN);
    let (r, _, effects) = run(&mem, 1, 0x5B);
    assert_eq!(stop(&r), STOP_AT_WORD);
    assert_eq!(writes(&effects), []);
}

#[test]
fn a_partial_store_stops_at_the_word() {
    let mem = memory(&[STB], RegisterWord::UNKNOWN);
    let (r, _, effects) = run(&mem, 1, 0x5A);
    assert_eq!(stop(&r), STOP_AT_WORD);
    assert_eq!(writes(&effects), []);
}

/// [CBE-Handbook p:90 s:4.2.6.8] A store's table lookup sets C from 0 to 1 unless the access takes a storage exception.
#[test]
fn an_accepted_store_through_an_entry_with_c_0_writes_c() {
    let mem = memory(&[STW], RegisterWord::UNKNOWN);
    let (r, _, effects) = run(&mem, 1, 0x5A);
    assert_eq!(r.fault, None);
    assert_eq!(writes(&effects), [(SPAN + OFFSET, vec![0, 0, 0, 0x5A])]);
    let c_at = HTAB + ((VSID ^ PAGE) & 0x7FF) * 128 + PTE_C_BYTE as u64;
    let c_writes: Vec<Vec<u8>> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == c_at => {
                Some(bytes.bytes().to_vec())
            }
            _ => None,
        })
        .collect();
    let c_byte = ((SPAN | PTE_LOW) | PTE_C).to_be_bytes()[PTE_C_BYTE - 8];
    assert_eq!(c_writes, [vec![c_byte]]);
}
