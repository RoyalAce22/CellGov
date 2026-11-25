//! Caching-inhibited storage refuses some stores at any alignment. Such a
//! store takes the alignment interrupt at an aligned operand in a writable
//! page whose entry has C = 0. Its table searches publish their R writes
//! and no C write. This is true in these cases:
//!
//! - the store stops in its batch;
//! - the batch ends ahead of the store, and the next batch takes the
//!   interrupt.

use super::*;

/// Doubleword 1 of page 4's entry: caching-inhibited and read/write, with
/// R = C = 0.
const PAGE_D1: u64 = PAGE_RA | PTE_I | 0b10;
/// Doubleword 1 of page 5's entry: cacheable and read/write, with
/// R = C = 0.
const NEXT_D1: u64 = NEXT_RA | 0b10;
/// Page 6, a second effective page over the real page of page 4 or page 5.
const ALIAS: u64 = 6;

/// `dcbz ra,rb`.
fn dcbz(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11) | (1014 << 1)
}

/// `stwcx. rs,0,rb`.
fn stwcx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (150 << 1) | 1
}

/// `stdcx. rs,0,rb`.
fn stdcx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (214 << 1) | 1
}

/// `stmw rs,d(ra)`.
fn stmw(rs: u32, d: u16, ra: u32) -> u32 {
    (47 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `stswi rs,ra,nb`.
fn stswi(rs: u32, ra: u32, nb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (nb << 11) | (725 << 1)
}

/// `stswx rs,0,rb`.
fn stswx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (661 << 1)
}

/// Commits the write intents of `effects` to `mem`.
fn commit(mem: &mut GuestMemory, effects: &[Effect]) {
    for e in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = e {
            put(mem, range.start().raw(), bytes.bytes());
        }
    }
}

/// Runs `insn` at page 4's first byte in one batch, with XER giving a
/// string store 4 bytes.
///
/// Asserts these facts:
/// - the store takes the alignment interrupt;
/// - the batch publishes the R write of page 4's entry and reads its
///   group;
/// - the batch publishes no C write to page 4's entry.
///
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup sets C = 0 to 1 unless the access takes a storage exception.
/// [PPC-Book3 p:26 s:4.2.1] A storage exception is one that causes, among others, an Alignment interrupt.
fn assert_stop_keeps_r_and_drops_c(insn: u32) {
    let mem = memory(&[insn], PAGE_D1, None);
    let mut unit = kernel_unit();
    unit.state_mut().set_xer(4);
    let (r, effects) = run(&mut unit, &mem, 1);
    let case = format!("insn {insn:#x}");
    assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}");
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        vec![r_byte(PAGE_D1)],
        "{case}"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        Vec::<u8>::new(),
        "{case}"
    );
    assert!(
        table_reads(&effects).contains(&(entry(PAGE), GROUP)),
        "{case}: {:x?}",
        table_reads(&effects)
    );
}

/// [CBE-Handbook p:253 s:9.5.8] A dcbz in caching-inhibited storage takes the alignment interrupt.
#[test]
fn an_aligned_dcbz_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(dcbz(0, 5));
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_stmw_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(stmw(30, 0, 5));
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_stswi_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(stswi(4, 5, 4));
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_stswx_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(stswx(4, 5));
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_stwcx_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(stwcx(4, 5));
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_stdcx_in_inhibited_storage_publishes_its_r_write_and_no_c_write() {
    assert_stop_keeps_r_and_drops_c(stdcx(4, 5));
}

/// Runs `unit` for the two batches of an aligned stw through page 6 and
/// the store after it. The function commits each batch's writes to `mem`
/// and returns the effects of both batches.
///
/// Asserts these facts:
/// - the stw retires in the first batch, and the batch ends ahead of the
///   store;
/// - the store takes the alignment interrupt in the second batch.
fn run_ended_batch(
    unit: &mut PpuExecutionUnit,
    mem: &mut GuestMemory,
    case: &str,
) -> [Vec<Effect>; 2] {
    let (_, first) = run(unit, mem, 2);
    assert_eq!(unit.retired_instructions(), 1, "{case}: the store waits");
    commit(mem, &first);
    let (r, second) = run(unit, mem, 1);
    commit(mem, &second);
    assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}: batch 2");
    [first, second]
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception.
/// [PPC-Book3 p:26 s:4.2.1] A storage exception is one that causes, among others, an Alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] A dcbz in caching-inhibited storage takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_inhibited_store_whose_batch_ends_ahead_of_it_publishes_its_r_write_and_no_c_write() {
    // Page 6 maps page 4's real page, caching-inhibited with R and C set.
    // The stw writes through page 6. The translation of page 4 then
    // aliases page 6, so the batch ends ahead of the store.
    let insns = [
        dcbz(0, 5),
        stwcx(4, 5),
        stdcx(4, 5),
        stmw(30, 0, 5),
        stswi(4, 5, 4),
        stswx(4, 5),
    ];
    for insn in insns {
        let mut mem = memory(&[stw(8, 0, 9), insn], PAGE_D1, None);
        map_page(&mut mem, ALIAS, PAGE_RA | PTE_I | PTE_R | PTE_C | 0b10);
        let mut unit = kernel_unit();
        unit.state_mut().set_xer(4);
        unit.state_mut().set_gpr(9, SEGMENT + (ALIAS << 12));
        let case = format!("insn {insn:#x}");
        let [first, second] = run_ended_batch(&mut unit, &mut mem, &case);
        assert_eq!(
            entry_byte_writes(&first, PAGE, PTE_R_BYTE),
            vec![r_byte(PAGE_D1)],
            "{case}: batch 1 R"
        );
        for (batch, effects) in [(1, &first), (2, &second)] {
            assert_eq!(
                entry_byte_writes(effects, PAGE, PTE_C_BYTE),
                Vec::<u8>::new(),
                "{case}: batch {batch} C"
            );
        }
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception.
/// [PPC-Book3 p:26 s:4.2.1] A storage exception is one that causes, among others, an Alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_inhibited_store_whose_batch_ends_at_its_second_page_publishes_r_writes_and_no_c_write(
) {
    // Each operand is the last word of page 4 and the first word of page
    // 5. Page 6 maps page 5's real page with R and C set. The stw writes
    // through page 6. The store writes C in page 4's entry, and then the
    // translation of page 5 aliases page 6, so the batch ends ahead of
    // the store.
    let insns = [stmw(30, 0xFFC, 5), stswi(4, 10, 8), stswx(4, 10)];
    for insn in insns {
        let mut mem = memory(&[stw(8, 0, 9), insn], PAGE_D1, Some(NEXT_D1));
        map_page(&mut mem, ALIAS, NEXT_RA | PTE_R | PTE_C | 0b10);
        let mut unit = kernel_unit();
        unit.state_mut().set_xer(8);
        unit.state_mut().set_gpr(9, SEGMENT + (ALIAS << 12));
        unit.state_mut().set_gpr(10, PAGE_EA + 0xFFC);
        let case = format!("insn {insn:#x}");
        let [first, second] = run_ended_batch(&mut unit, &mut mem, &case);
        assert_eq!(
            entry_byte_writes(&first, PAGE, PTE_R_BYTE),
            vec![r_byte(PAGE_D1)],
            "{case}: batch 1 page 4 R"
        );
        assert_eq!(
            entry_byte_writes(&first, NEXT, PTE_R_BYTE),
            vec![r_byte(NEXT_D1)],
            "{case}: batch 1 page 5 R"
        );
        for (batch, effects) in [(1, &first), (2, &second)] {
            for index in [PAGE, NEXT] {
                assert_eq!(
                    entry_byte_writes(effects, index, PTE_C_BYTE),
                    Vec::<u8>::new(),
                    "{case}: batch {batch} page {index} C"
                );
            }
        }
    }
}
