//! Guest stack-walk caller classification and branch-encoding round-trips.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::ppc_isa::{PPC_BCCTR_XO, PPC_BCLR_XO};
use cellgov_time::Budget;

fn rt_with_layout() -> Runtime {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
        Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
    ])
    .unwrap();
    Runtime::new(mem, Budget::new(1), 100)
}

/// Encode `bl <offset>` per [PPC-Book1 p:24 s:2.4.1 Branch
/// Instructions] I-form: opcode 18, LI in bits 6..29 (24-bit signed
/// byte displacement >> 2), AA bit 30, LK bit 31.
fn encode_bl(offset: i32) -> u32 {
    debug_assert!(
        offset & 3 == 0,
        "bl offset must be 4-byte aligned, got {offset:#x}"
    );
    debug_assert!(
        (-(1i32 << 25)..(1i32 << 25)).contains(&offset),
        "bl offset {offset:#x} out of LI range +-32 MiB"
    );
    let li = ((offset >> 2) as u32) & 0x00FF_FFFF;
    (18u32 << 26) | (li << 2) | 0b01
}

/// Encode `bcctrl <bo>,<bi>` per [PPC-Book1 p:25 s:Branch
/// Conditional to Count Register]: opcode 19, BO at bits 6..10, BI
/// at 11..15, XO=528 at 21..30, LK=31.
fn encode_bcctrl(bo: u8, bi: u8) -> u32 {
    debug_assert!(bo < 32);
    debug_assert!(bi < 32);
    (19u32 << 26) | ((bo as u32) << 21) | ((bi as u32) << 16) | (PPC_BCCTR_XO << 1) | 1
}

/// Encode `blrl` per [PPC-Book1 p:25 s:Branch Conditional to Link
/// Register]: opcode 19, BO=20 (branch always), BI=0, XO=16, LK=1.
fn encode_blrl() -> u32 {
    (19u32 << 26) | (20u32 << 21) | (PPC_BCLR_XO << 1) | 1
}

fn encode_b_nolink(offset: i32) -> u32 {
    debug_assert!(offset & 3 == 0);
    let li = ((offset >> 2) as u32) & 0x00FF_FFFF;
    (18u32 << 26) | (li << 2)
}

fn write_bytes(rt: &mut Runtime, addr: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
    rt.memory_mut().apply_commit(range, bytes).unwrap();
}

fn write_u32_be(rt: &mut Runtime, addr: u64, word: u32) {
    write_bytes(rt, addr, &word.to_be_bytes());
}

fn write_u64_be(rt: &mut Runtime, addr: u64, value: u64) {
    write_bytes(rt, addr, &value.to_be_bytes());
}

fn fault_regs_with_r1(r1: u64) -> FaultRegisterDump {
    let mut regs = FaultRegisterDump {
        gprs: [0; 32],
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    };
    regs.gprs[1] = r1;
    regs
}

fn count_frame_lines(out: &str) -> usize {
    out.lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .count()
}

#[test]
fn user_text_floor_const_matches_callback_dispatch_zone() {
    // Trampoline scratch zone is 0..0x10000; floor must equal its upper bound.
    assert_eq!(PS3_USER_TEXT_FLOOR, 0x0001_0000);
}

#[test]
fn saved_lr_call_kind_rejects_high_half() {
    let rt = rt_with_layout();
    assert_eq!(saved_lr_call_kind(&rt, 0x1_0000_0000), None);
    assert_eq!(saved_lr_call_kind(&rt, 0xDEAD_BEEF_0010_0004), None);
}

#[test]
fn saved_lr_call_kind_rejects_misaligned() {
    let rt = rt_with_layout();
    for off in 1..=3 {
        assert_eq!(saved_lr_call_kind(&rt, 0x0010_0000 + off), None);
    }
}

#[test]
fn saved_lr_call_kind_rejects_below_text_floor() {
    let rt = rt_with_layout();
    assert_eq!(saved_lr_call_kind(&rt, 0), None);
    assert_eq!(saved_lr_call_kind(&rt, 0xFFFC), None);
    assert_eq!(saved_lr_call_kind(&rt, PS3_USER_TEXT_FLOOR - 4), None);
}

#[test]
fn saved_lr_call_kind_classifies_real_bl_caller() {
    let mut rt = rt_with_layout();
    let call_pc = 0x0010_0000u64;
    write_u32_be(&mut rt, call_pc, encode_bl(0x100));
    assert_eq!(saved_lr_call_kind(&rt, call_pc + 4), Some(CallKind::Bl));
}

#[test]
fn encode_bl_roundtrips_through_decoder() {
    for offset in [4, 0x100, -0x200, -4, (1 << 25) - 4, -(1 << 25)] {
        let raw = encode_bl(offset);
        match decode::decode(raw).unwrap() {
            PpuInstruction::B { link: true, .. } => {}
            other => panic!("offset {offset:#x} -> {raw:#010x} decoded as {other:?}"),
        }
    }
}

#[test]
fn encode_bcctrl_with_valid_bo_roundtrips() {
    let raw = encode_bcctrl(20, 0);
    match decode::decode(raw).unwrap() {
        PpuInstruction::Bcctr {
            bo,
            bi: 0,
            link: true,
        } => assert_eq!(bo, 20),
        other => panic!("expected Bcctr{{link:true}}, got {other:?}"),
    }
}

#[test]
fn classify_call_at_recognizes_bl() {
    let mut rt = rt_with_layout();
    write_u32_be(&mut rt, 0x0010_0000, encode_bl(0x100));
    assert_eq!(classify_call_at(&rt, 0x0010_0000), Some(CallKind::Bl));
}

#[test]
fn classify_call_at_recognizes_valid_bcctrl() {
    let mut rt = rt_with_layout();
    write_u32_be(&mut rt, 0x0010_0000, encode_bcctrl(20, 0));
    assert_eq!(classify_call_at(&rt, 0x0010_0000), Some(CallKind::Bcctrl));
}

#[test]
fn classify_call_at_rejects_invalid_form_bcctrl() {
    let mut rt = rt_with_layout();
    for bo in [0u8, 16, 8, 24] {
        write_u32_be(&mut rt, 0x0010_0000, encode_bcctrl(bo, 0));
        assert_eq!(
            classify_call_at(&rt, 0x0010_0000),
            None,
            "BO={bo:#b} should be invalid (BO2=0)"
        );
    }
}

#[test]
fn classify_call_at_recognizes_blrl() {
    let mut rt = rt_with_layout();
    write_u32_be(&mut rt, 0x0010_0000, encode_blrl());
    assert_eq!(classify_call_at(&rt, 0x0010_0000), Some(CallKind::Bclrl));
}

#[test]
fn classify_call_at_rejects_non_link_branch() {
    let mut rt = rt_with_layout();
    write_u32_be(&mut rt, 0x0010_0000, encode_b_nolink(0x100));
    assert_eq!(classify_call_at(&rt, 0x0010_0000), None);
}

#[test]
fn classify_call_at_rejects_zero_word() {
    let rt = rt_with_layout();
    assert_eq!(classify_call_at(&rt, 0x0010_0000), None);
}

#[test]
fn classify_call_at_returns_none_on_unmapped() {
    let rt = rt_with_layout();
    assert_eq!(classify_call_at(&rt, 0x8000_0000), None);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "misaligned")]
fn classify_call_at_debug_asserts_alignment() {
    let rt = rt_with_layout();
    let _ = classify_call_at(&rt, 0x0010_0001);
}

/// Layout per [AltiVec-PIM p:34 s:3]:
///   sp_a (innermost) -> sp_b -> sp_c -> NULL.
/// sp_a's saved LR lives at sp_b+16; sp_b's saved LR at sp_c+16.
#[test]
fn back_chain_walk_recovers_callers_after_null_bcctr() {
    let mut rt = rt_with_layout();
    let inner_caller_pc = 0x0020_0000u64;
    let outer_caller_pc = 0x0020_1000u64;
    write_u32_be(&mut rt, inner_caller_pc, encode_bcctrl(20, 0));
    write_u32_be(&mut rt, outer_caller_pc, encode_bl(0x100));

    let sp_a = 0xD000_0080u64;
    let sp_b = 0xD000_0100u64;
    let sp_c = 0xD000_0180u64;
    write_u64_be(&mut rt, sp_a, sp_b);
    write_u64_be(&mut rt, sp_b, sp_c);
    write_u64_be(&mut rt, sp_c, 0);
    write_u64_be(&mut rt, sp_b + 16, inner_caller_pc + 4);
    write_u64_be(&mut rt, sp_c + 16, outer_caller_pc + 4);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp_a));

    assert!(out.contains("back-chain walk"), "expected block in {out}");
    assert!(
        out.contains(&format!("saved_lr=0x{:016x}", inner_caller_pc + 4)),
        "missing inner saved_lr in {out}",
    );
    assert!(
        out.contains(&format!("saved_lr=0x{:016x}", outer_caller_pc + 4)),
        "missing outer saved_lr in {out}",
    );
    assert!(out.contains("via bcctrl"), "missing bcctrl tag in {out}");
    assert!(out.contains("via bl"), "missing bl tag in {out}");
    assert!(
        out.contains("NULL back-chain"),
        "expected NULL termination annotation in {out}",
    );
    assert_eq!(count_frame_lines(&out), 3, "frame count drift in {out}");
}

#[test]
fn back_chain_walk_terminates_on_non_increasing_sp() {
    let mut rt = rt_with_layout();
    let sp_a = 0xD000_0100u64;
    let sp_b = 0xD000_0080u64;
    write_u64_be(&mut rt, sp_a, sp_b);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp_a));
    assert!(
        out.contains("stack walk skipped")
            && out.contains("implausible")
            && out.contains("non-increasing"),
        "expected skipped-implausible annotation, got {out}",
    );
}

#[test]
fn back_chain_walk_terminates_on_below_floor_back_chain() {
    let mut rt = rt_with_layout();
    let sp = 0xD000_0080u64;
    write_u64_be(&mut rt, sp, 0x42);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp));
    assert!(
        out.contains("stack walk skipped") && out.contains("implausible"),
        "expected skipped-implausible annotation, got {out}",
    );
}

#[test]
fn append_stack_walk_dumps_bytes_on_invalid_back_chain_frame_0() {
    let mut rt = rt_with_layout();
    let sp = 0xD000_0080u64;
    write_u64_be(&mut rt, sp, 0xCAFE_BABE);
    write_u64_be(&mut rt, sp + 16, 0xDEAD_BEEF_0000_0001);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp));
    assert!(out.contains("stack walk skipped"), "got {out}");
    assert!(
        out.contains(&format!("bytes at sp=0x{sp:016x} (back-chain at +0):")),
        "got {out}",
    );
    assert!(out.contains("ca fe ba be"), "got {out}");
    assert!(out.contains("de ad be ef"), "got {out}");
}

#[test]
fn append_stack_walk_dumps_bytes_on_invalid_back_chain_mid_walk() {
    let mut rt = rt_with_layout();
    let sp_a = 0xD000_0080u64;
    let sp_b = 0xD000_0100u64;
    let bl_pc = 0x0020_0000u64;
    write_u32_be(&mut rt, bl_pc, encode_bl(0x100));
    write_u64_be(&mut rt, sp_a, sp_b);
    write_u64_be(&mut rt, sp_b + 16, bl_pc + 4);
    write_u64_be(&mut rt, sp_b, 0x10);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp_a));
    assert!(out.contains("back-chain walk"), "got {out}");
    assert!(out.contains("via bl"), "got {out}");
    assert!(out.contains("implausible"), "got {out}");
    assert!(
        out.contains(&format!("bytes at sp=0x{sp_b:016x}")),
        "got {out}",
    );
}

#[test]
fn append_stack_walk_omits_byte_dump_on_unmapped_r1() {
    let rt = rt_with_layout();
    let r1 = 0x8000_0000u64;
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(r1));
    assert!(out.contains("stack walk skipped"), "got {out}");
    assert!(!out.contains("bytes at sp="), "unexpected dump in {out}");
}

#[test]
fn back_chain_walk_caps_at_max_frames() {
    let mut rt = rt_with_layout();
    let bl_pc = 0x0020_0000u64;
    write_u32_be(&mut rt, bl_pc, encode_bl(0x100));
    let frame_size: u64 = 64;
    let base = 0xD000_0000u64;
    for i in 0..(MAX_BACK_CHAIN_FRAMES as u64 + 5) {
        let sp = base + i * frame_size;
        let next_sp = base + (i + 1) * frame_size;
        write_u64_be(&mut rt, sp, next_sp);
        write_u64_be(&mut rt, sp + 16, bl_pc + 4);
    }
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(base));
    assert!(
        out.contains("max frames reached"),
        "expected max-frames termination, got {out}",
    );
    assert_eq!(
        count_frame_lines(&out),
        MAX_BACK_CHAIN_FRAMES,
        "frame count drift: got {out}",
    );
}

#[test]
fn back_chain_walk_skipped_message_on_unmapped_r1() {
    let rt = rt_with_layout();
    let r1 = 0x8000_0000u64;
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(r1));
    assert!(
        out.contains("stack walk skipped") && out.contains("unmapped"),
        "expected skipped-unmapped annotation, got {out}",
    );
}

#[test]
fn append_stack_walk_skips_when_r1_below_text_floor() {
    let rt = rt_with_layout();
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(0xFFF));
    assert!(
        out.is_empty(),
        "expected silence below text floor, got {out}"
    );
}

#[test]
fn append_stack_walk_skips_when_r1_is_zero() {
    let rt = rt_with_layout();
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(0));
    assert!(out.is_empty(), "expected silence on r1=0, got {out}");
}

#[test]
fn back_chain_walk_reports_frame_with_not_a_call_when_saved_lr_is_junk() {
    let mut rt = rt_with_layout();
    let sp_a = 0xD000_0080u64;
    let sp_b = 0xD000_0100u64;
    write_u64_be(&mut rt, sp_a, sp_b);
    write_u64_be(&mut rt, sp_b, 0);
    write_u64_be(&mut rt, sp_b + 16, 0x0010_0000);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp_a));
    assert_eq!(count_frame_lines(&out), 2, "got {out}");
    assert!(out.contains("via not-a-call"), "got {out}");
    assert!(out.contains("saved_lr=0x0000000000100000"), "got {out}");
}

#[test]
fn back_chain_walk_preserves_high_half_in_corrupt_saved_lr() {
    let mut rt = rt_with_layout();
    let sp_a = 0xD000_0080u64;
    let sp_b = 0xD000_0100u64;
    write_u64_be(&mut rt, sp_a, sp_b);
    write_u64_be(&mut rt, sp_b, 0);
    write_u64_be(&mut rt, sp_b + 16, 0xDEAD_BEEF_0010_0004);

    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(sp_a));
    assert_eq!(count_frame_lines(&out), 2);
    assert!(out.contains("via not-a-call"), "got {out}");
    assert!(
        out.contains("saved_lr=0xdeadbeef00100004"),
        "high bits truncated in {out}",
    );
}

#[test]
fn back_chain_walk_traverses_realistic_5_frame_chain_to_null() {
    let mut rt = rt_with_layout();
    let bl_pc = 0x0020_0000u64;
    write_u32_be(&mut rt, bl_pc, encode_bl(0x100));

    let frame_size: u64 = 96;
    let base = 0xD000_0000u64;
    const N: u64 = 5;
    for i in 0..N {
        let sp = base + i * frame_size;
        let next_sp = if i == N - 1 {
            0
        } else {
            base + (i + 1) * frame_size
        };
        write_u64_be(&mut rt, sp, next_sp);
        write_u64_be(&mut rt, sp + 16, bl_pc + 4);
    }
    let mut out = String::new();
    append_stack_walk(&mut out, &rt, &fault_regs_with_r1(base));
    assert!(out.contains("NULL back-chain"), "got {out}");
    assert_eq!(count_frame_lines(&out), N as usize, "got {out}");
    assert_eq!(out.matches("via bl").count(), (N - 1) as usize, "got {out}");
    assert_eq!(out.matches("via not-a-call").count(), 1, "got {out}");
}
