//! Stack-frame walker for `outcome: FAULT` diagnostics.
//!
//! ABI back-chain walk per [CBE-Handbook p:398 s:14.3.1.3 Figure 14-3
//! PPE 64-Bit Standard Stack Frame] and [AltiVec-PIM p:34 s:3 ABI
//! prologue example] (`stw r0, 4(sp) # in caller's frame` for 32-bit;
//! the 64-bit analogue is `std r0, 16(r1)`):
//!
//! - SP+0 holds the back-chain pointer (caller's SP).
//! - The function's own saved LR lives at the **caller's** SP+16 -- the
//!   prologue runs `mflr r0; std r0, 16(r1)` BEFORE `stdu r1,
//!   -frame_size(r1)`, so the store hits OLD_r1+16 = caller's frame
//!   top + 16. After stdu, that slot is at `back_chain + 16`.
//! - SP+16 of the *current* frame is reserved for THIS function's
//!   callees to populate (their own LR-save). Reading it gets stale
//!   data for the most recent callee that returned.
//!
//! The walker reads each frame's saved LR at `next_sp + 16` where
//! `next_sp = *(sp+0)` is the back-chain. Frame 0 (the leaf at fault
//! time) reports its caller's return address; the function the fault
//! occurred *in* lives in the LR register, which the surrounding
//! `format_fault` register dump already shows.
//!
//! Walks terminate on NULL back chain (the initial frame), unmapped
//! read, an implausible back-chain pointer (non-increasing or
//! below-floor -- monotonic-SP also subsumes cycle detection), or
//! `MAX_BACK_CHAIN_FRAMES`.

use cellgov_core::Runtime;
use cellgov_exec::FaultRegisterDump;
use cellgov_ppu::decode;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ps3_abi::ppc_isa::PPC_BO_BIT2 as BO_BIT2;
use cellgov_ps3_abi::process_address_space::PS3_USER_TEXT_FLOOR;

const MAX_BACK_CHAIN_FRAMES: usize = 32;

/// Empty output means r1 sits below the user-text floor (NULL or trampoline-scratch).
pub(super) fn append_stack_walk(out: &mut String, rt: &Runtime, regs: &FaultRegisterDump) {
    let r1 = regs.gprs[1];
    if r1 < PS3_USER_TEXT_FLOOR {
        return;
    }

    let walk = walk_back_chain(rt, r1);
    if walk.frames.is_empty() {
        out.push_str(&format!(
            "\n  stack walk skipped: r1=0x{r1:016x} -- {}",
            walk.terminated.as_str(),
        ));
        if walk.terminated == Termination::InvalidBackChain {
            if let Some(sp) = walk.last_sp_visited {
                append_back_chain_byte_dump(out, rt, sp);
            }
        }
        return;
    }
    out.push_str(&format!(
        "\n  back-chain walk (r1=0x{r1:016x}, {} frame(s), terminated: {}):",
        walk.frames.len(),
        walk.terminated.as_str(),
    ));
    for (idx, f) in walk.frames.iter().enumerate() {
        let kind = f.call_kind.map(CallKind::as_str).unwrap_or("not-a-call");
        out.push_str(&format!(
            "\n    #{idx:>2} sp=0x{:016x}  saved_lr=0x{:016x}  via {kind}",
            f.sp, f.saved_lr,
        ));
    }
    if walk.terminated == Termination::InvalidBackChain {
        if let Some(sp) = walk.last_sp_visited {
            append_back_chain_byte_dump(out, rt, sp);
        }
    }
}

/// 16 quadwords (128 bytes) at `sp`; silent on read failure.
fn append_back_chain_byte_dump(out: &mut String, rt: &Runtime, sp: u64) {
    const DUMP_LEN: u64 = 128;
    let Some(range) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(sp), DUMP_LEN) else {
        return;
    };
    let Some(slice) = rt.memory().read(range) else {
        return;
    };
    out.push_str(&format!(
        "\n    bytes at sp=0x{sp:016x} (back-chain at +0):"
    ));
    for (row, chunk) in slice.chunks(16).enumerate() {
        out.push_str(&format!("\n      +0x{:03x}:", row * 16));
        for b in chunk {
            out.push_str(&format!(" {b:02x}"));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Bl,
    Bcl,
    Bcctrl,
    Bclrl,
}

impl CallKind {
    fn as_str(self) -> &'static str {
        match self {
            CallKind::Bl => "bl",
            CallKind::Bcl => "bcl",
            CallKind::Bcctrl => "bcctrl",
            CallKind::Bclrl => "bclrl",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BackChainFrame {
    sp: u64,
    saved_lr: u64,
    /// `None` means the saved-LR slot does not point at a call site;
    /// useful for diagnosing prologue-corruption shapes.
    call_kind: Option<CallKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Termination {
    NullBackChain,
    /// Back-chain pointer is below `PS3_USER_TEXT_FLOOR`, equal to or
    /// below the current SP (the chain must climb), or otherwise
    /// outside the stack region. The monotonic-SP rule subsumes cycle
    /// detection: a revisit can only happen if the chain went
    /// backwards.
    InvalidBackChain,
    UnmappedRead,
    MaxFrames,
}

impl Termination {
    fn as_str(self) -> &'static str {
        match self {
            Termination::NullBackChain => "NULL back-chain (initial frame)",
            Termination::InvalidBackChain => {
                "back-chain pointer is implausible (non-increasing, below floor, or cycle)"
            }
            Termination::UnmappedRead => "unmapped or region-straddle read",
            Termination::MaxFrames => "max frames reached",
        }
    }
}

struct BackChainWalk {
    frames: Vec<BackChainFrame>,
    terminated: Termination,
    /// For InvalidBackChain mid-walk: the SP whose back-chain pointed somewhere
    /// implausible. For frame 0 it equals `r1`. None on NullBackChain/MaxFrames.
    last_sp_visited: Option<u64>,
}

/// Walk PPC64 ELFv1 back-chain anchored at `r1` per
/// [CBE-Handbook p:398 s:14.3.1.3 Figure 14-3] and [AltiVec-PIM p:34 s:3].
/// Each frame's saved LR lives at its caller's SP+16 (not its own), so the walk reads
/// `read_u64(next_sp + 16)` after following the back-chain. The initial frame
/// (NULL back-chain) is pushed with `saved_lr = 0` / `call_kind = None`.
fn walk_back_chain(rt: &Runtime, mut sp: u64) -> BackChainWalk {
    let mut frames: Vec<BackChainFrame> = Vec::new();

    for _ in 0..MAX_BACK_CHAIN_FRAMES {
        let Some(next_sp) = read_u64(rt, sp) else {
            return BackChainWalk {
                frames,
                terminated: Termination::UnmappedRead,
                last_sp_visited: Some(sp),
            };
        };

        if next_sp == 0 {
            frames.push(BackChainFrame {
                sp,
                saved_lr: 0,
                call_kind: None,
            });
            return BackChainWalk {
                frames,
                terminated: Termination::NullBackChain,
                last_sp_visited: None,
            };
        }
        if next_sp <= sp || next_sp < PS3_USER_TEXT_FLOOR {
            return BackChainWalk {
                frames,
                terminated: Termination::InvalidBackChain,
                last_sp_visited: Some(sp),
            };
        }

        // Saved LR at next_sp + 16 (caller's r1 + 16): where THIS frame's prologue stored it.
        let Some(saved_lr_addr) = next_sp.checked_add(16) else {
            return BackChainWalk {
                frames,
                terminated: Termination::UnmappedRead,
                last_sp_visited: Some(sp),
            };
        };
        let Some(saved_lr_raw) = read_u64(rt, saved_lr_addr) else {
            return BackChainWalk {
                frames,
                terminated: Termination::UnmappedRead,
                last_sp_visited: Some(sp),
            };
        };

        let call_kind = saved_lr_call_kind(rt, saved_lr_raw);
        // Store raw u64 so corrupt high bits print in full; `via not-a-call` flags them.
        frames.push(BackChainFrame {
            sp,
            saved_lr: saved_lr_raw,
            call_kind,
        });

        sp = next_sp;
    }
    BackChainWalk {
        frames,
        terminated: Termination::MaxFrames,
        last_sp_visited: None,
    }
}

/// Rejects high-32-bit-set, misaligned, below-floor, and values whose
/// preceding word does not decode to a call-with-link.
fn saved_lr_call_kind(rt: &Runtime, saved_lr_raw: u64) -> Option<CallKind> {
    if saved_lr_raw >> 32 != 0 {
        return None;
    }
    let saved_lr = saved_lr_raw & 0xFFFF_FFFF;
    if saved_lr < PS3_USER_TEXT_FLOOR || saved_lr & 3 != 0 {
        return None;
    }
    classify_call_at(rt, saved_lr.wrapping_sub(4))
}

fn read_u64(rt: &Runtime, addr: u64) -> Option<u64> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 8)?;
    let bytes = rt.memory().read(range)?;
    Some(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_u32(rt: &Runtime, addr: u64) -> Option<u32> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4)?;
    let bytes = rt.memory().read(range)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Rejects bcctr's invalid form (BO2=0) per
/// [PPC-Book1 p:25 s:Branch Conditional to Count Register].
fn classify_call_at(rt: &Runtime, addr: u64) -> Option<CallKind> {
    debug_assert!(
        addr & 3 == 0,
        "classify_call_at called with misaligned addr=0x{addr:x}"
    );
    let raw = read_u32(rt, addr)?;
    let insn = decode::decode(raw).ok()?;
    match insn {
        PpuInstruction::B { link: true, .. } => Some(CallKind::Bl),
        PpuInstruction::Bc { link: true, .. } => Some(CallKind::Bcl),
        // [PPC-Book1 p:25 s:Branch Conditional to Count Register] BO2=0 is invalid for bcctr.
        PpuInstruction::Bcctr { bo, link: true, .. } if bo & BO_BIT2 != 0 => Some(CallKind::Bcctrl),
        PpuInstruction::Bclr { link: true, .. } => Some(CallKind::Bclrl),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
    use cellgov_ps3_abi::ppc_isa::{PPC_BCCTR_XO as BCCTR_XO, PPC_BCLR_XO as BCLR_XO};
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
        (19u32 << 26) | ((bo as u32) << 21) | ((bi as u32) << 16) | (BCCTR_XO << 1) | 1
    }

    /// Encode `blrl` per [PPC-Book1 p:25 s:Branch Conditional to Link
    /// Register]: opcode 19, BO=20 (branch always), BI=0, XO=16, LK=1.
    fn encode_blrl() -> u32 {
        (19u32 << 26) | (20u32 << 21) | (BCLR_XO << 1) | 1
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
}
