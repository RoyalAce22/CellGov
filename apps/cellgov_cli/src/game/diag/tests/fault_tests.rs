//! Commit-fault and deadlock diagnostic formatting with PC-ring context.

use super::*;
use cellgov_exec::FaultRegisterDump;
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_time::Budget;

fn rt_with_layout() -> Runtime {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
        Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
    ])
    .unwrap();
    Runtime::new(mem, Budget::new(1), 100)
}

fn write_bytes(rt: &mut Runtime, addr: u64, bytes: &[u8]) {
    let range =
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64).unwrap();
    rt.memory_mut().apply_commit(range, bytes).unwrap();
}

fn fault_regs() -> FaultRegisterDump {
    FaultRegisterDump {
        gprs: [0; 32],
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    }
}

#[test]
fn format_commit_fault_includes_error_and_step_and_pc_ring() {
    let rt = rt_with_layout();
    let err = CommitError::PayloadLengthMismatch { effect_index: 3 };
    let mut cursor = RingCursor::new(PC_RING_SIZE);
    let mut ring = [0u64; PC_RING_SIZE];
    for pc in [0x0010_0000u64, 0x0010_0004, 0x0010_0008] {
        let idx = cursor.record();
        ring[idx] = pc;
    }
    let out = format_commit_fault(
        &rt,
        &err,
        1234,
        cellgov_event::UnitId::new(0),
        &ring,
        &cursor,
    );
    assert!(out.starts_with("COMMIT_FAULT at step 1234"), "got {out}");
    assert!(out.contains("PayloadLengthMismatch"), "got {out}");
    assert!(out.contains("last 3 PCs:"), "got {out}");
    assert!(out.contains("0x00100000"), "got {out}");
}

#[test]
fn format_deadlock_with_empty_registry_flags_drift() {
    let rt = rt_with_layout();
    let cursor = RingCursor::new(PC_RING_SIZE);
    let ring = [0u64; PC_RING_SIZE];
    let out = format_deadlock(&rt, 99, &ring, &cursor);
    assert!(out.starts_with("DEADLOCK after 99 steps:"), "got {out}");
    assert!(
        out.contains("no Blocked units in registry") && out.contains("AllBlocked"),
        "expected drift note in {out}",
    );
}

#[test]
fn format_deadlock_dumps_ppu_unit_with_lv2_reason_and_spu_unit_without() {
    use cellgov_lv2::{GuestBlockReason, PpuThreadAttrs, PpuThreadState};
    use cellgov_testkit::world::CountingUnit;

    let mut rt = rt_with_layout();
    let unit_a = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let unit_b = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));

    rt.registry_mut()
        .set_status_override(unit_a, UnitStatus::Blocked);
    rt.registry_mut()
        .set_status_override(unit_b, UnitStatus::Blocked);

    let attrs = PpuThreadAttrs {
        entry: 0x10_0000,
        arg: 0,
        stack_base: 0xD000_0000,
        stack_size: 0x10000,
        priority: 1000,
        tls_base: 0x0020_0000,
    };
    let ppu_id_a = rt
        .lv2_host_mut()
        .ppu_threads_mut()
        .create(unit_a, attrs)
        .unwrap();
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .get_mut(ppu_id_a)
        .unwrap()
        .state = PpuThreadState::Blocked(GuestBlockReason::WaitingOnLwMutex { id: 7 });
    // unit_b has no PpuThread record (models SPU/non-PPU shape).

    let cursor = RingCursor::new(PC_RING_SIZE);
    let ring = [0u64; PC_RING_SIZE];
    let out = format_deadlock(&rt, 42, &ring, &cursor);
    assert!(out.contains("DEADLOCK after 42 steps:"), "got {out}");
    assert!(
        out.contains(&format!("unit {} (PPU thread", unit_a.raw())),
        "PPU unit not labeled: {out}",
    );
    assert!(out.contains("WaitingOnLwMutex(id=7)"), "got {out}");
    assert!(
        out.contains(&format!("unit {} (no LV2 PPU thread record", unit_b.raw())),
        "SPU-shaped unit not labeled: {out}",
    );
    assert!(out.contains("2 blocked unit(s) total"), "got {out}");
}

#[test]
fn append_register_pointer_dump_emits_for_pointer_gprs_and_skips_zero() {
    let mut rt = rt_with_layout();
    write_bytes(&mut rt, 0x0010_0000, b"hello world\x00\x00\x00\x00\x00");
    let mut regs = fault_regs();
    regs.gprs[3] = 0x0010_0000;
    regs.gprs[4] = 0;

    let mut out = String::new();
    append_register_pointer_dump(&mut out, &rt, &regs);
    assert!(out.contains("register pointers:"), "got {out}");
    assert!(out.contains("[r3=0x0000000000100000 (main)]"), "got {out}");
    assert!(out.contains("\"hello world\""), "got {out}");
    assert!(!out.contains("[r4="), "zero r4 leaked: {out}");
}

#[test]
fn append_register_pointer_dump_handles_unmapped_pointer() {
    let rt = rt_with_layout();
    let mut regs = fault_regs();
    regs.gprs[7] = 0x8000_0000;

    let mut out = String::new();
    append_register_pointer_dump(&mut out, &rt, &regs);
    assert!(out.contains("[r7=0x0000000080000000"), "got {out}");
    assert!(out.contains("<unreadable>"), "got {out}");
}

#[test]
fn append_register_pointer_dump_includes_lr_and_ctr() {
    let mut rt = rt_with_layout();
    write_bytes(&mut rt, 0x0020_0000, &[0xab; 64]);
    let mut regs = fault_regs();
    regs.lr = 0x0020_0000;
    regs.ctr = 0x0020_0000;

    let mut out = String::new();
    append_register_pointer_dump(&mut out, &rt, &regs);
    assert!(out.contains("[LR=0x0000000000200000"), "got {out}");
    assert!(out.contains("[CTR=0x0000000000200000"), "got {out}");
}

#[test]
fn append_register_pointer_dump_emits_nothing_when_no_pointers() {
    let rt = rt_with_layout();
    let regs = fault_regs();

    let mut out = String::new();
    append_register_pointer_dump(&mut out, &rt, &regs);
    assert!(out.is_empty(), "expected silence, got {out}");
}

#[test]
fn append_explicit_mem_dump_renders_hex_and_ascii() {
    let mut rt = rt_with_layout();
    write_bytes(&mut rt, 0x0010_0000, b"ABCDEFGHIJKLMNOP\x00\x01\x02\x03");
    let ranges = [(0x0010_0000u64, 20u64)];

    let mut out = String::new();
    append_explicit_mem_dump(&mut out, &rt, &ranges);
    assert!(out.contains("explicit mem dumps:"), "got {out}");
    assert!(
        out.contains("mem[0x0000000000100000 (main), 20 bytes]:"),
        "got {out}"
    );
    // Hex bytes for "ABCDEFGHIJKLMNOP".
    assert!(
        out.contains("41 42 43 44 45 46 47 48"),
        "hex row missing in {out}"
    );
    assert!(out.contains("|ABCDEFGHIJKLMNOP|"), "got {out}");
}

#[test]
fn append_explicit_mem_dump_invalid_range_does_not_panic() {
    let rt = rt_with_layout();
    let ranges = [(u64::MAX, 64u64)];

    let mut out = String::new();
    append_explicit_mem_dump(&mut out, &rt, &ranges);
    assert!(out.contains("explicit mem dumps:"), "got {out}");
    assert!(out.contains("<invalid address range>"), "got {out}");
}

#[test]
fn append_explicit_mem_dump_unmapped_falls_back_to_unmapped_marker() {
    let rt = rt_with_layout();
    let ranges = [(0x8000_0000u64, 64u64)];

    let mut out = String::new();
    append_explicit_mem_dump(&mut out, &rt, &ranges);
    assert!(out.contains("<unmapped>"), "got {out}");
}

#[test]
fn append_explicit_mem_dump_partial_straddle_shows_prefix() {
    let mut rt = rt_with_layout();
    let buf = 0x4000_0000 - 8;
    write_bytes(&mut rt, buf, &[0x55; 8]);
    let ranges = [(buf, 32u64)];

    let mut out = String::new();
    append_explicit_mem_dump(&mut out, &rt, &ranges);
    assert!(out.contains("8/32 bytes (tail 24 unmapped)"), "got {out}");
    assert!(out.contains("55 55 55 55 55 55 55 55"), "got {out}");
}

#[test]
fn append_explicit_mem_dump_empty_ranges_emits_nothing() {
    let rt = rt_with_layout();
    let mut out = String::new();
    append_explicit_mem_dump(&mut out, &rt, &[]);
    assert!(out.is_empty(), "expected silence on empty, got {out}");
}
