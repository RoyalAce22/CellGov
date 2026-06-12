//! Stack-frame walker for `outcome: FAULT` diagnostics.
//!
//! ABI back-chain walk per [CBE-Handbook p:398 s:14.3.1.3 Figure 14-3
//! PPE 64-Bit Standard Stack Frame] and [AltiVec-PIM p:34 s:3]:
//! SP+0 holds the back-chain pointer (caller's SP), and the function's
//! own saved LR lives at the **caller's** SP+16 -- the prologue runs
//! `mflr r0; std r0, 16(r1)` BEFORE `stdu r1, -frame_size(r1)`, so the
//! walker reads saved LR at `next_sp + 16` where
//! `next_sp = *(sp+0)`.

use cellgov_core::Runtime;
use cellgov_exec::FaultRegisterDump;
use cellgov_ppu::decode;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ps3_abi::ppc_isa::PPC_BO_BIT2;
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
    /// Monotonic-SP rule subsumes cycle detection.
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

/// Walk PPC64 ELFv1 back-chain anchored at `r1`; the initial frame
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
        // Raw u64 so corrupt high bits print in full; `via not-a-call` flags them.
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
        PpuInstruction::Bcctr { bo, link: true, .. } if bo & PPC_BO_BIT2 != 0 => {
            Some(CallKind::Bcctrl)
        }
        PpuInstruction::Bclr { link: true, .. } => Some(CallKind::Bclrl),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/stack_walk_tests.rs"]
mod tests;
