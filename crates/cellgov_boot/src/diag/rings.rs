//! Renders the stepper's PC and syscall rings into a report.

use cellgov_core::{AddressSpaceId, Runtime};

use crate::step_loop::{PcRing, SyscallRing};

use super::exit::ProcessExitInfo;
use super::{fetch_raw_at, format_hle_idx};

/// Empty for the boot space, so a single-process report is unchanged.
fn space_tag(space: AddressSpaceId) -> String {
    if space == AddressSpaceId::BOOT {
        String::new()
    } else {
        format!("  space={}", space.raw())
    }
}

pub(crate) fn append_pc_ring_with_decode(out: &mut String, rt: &Runtime, pc_ring: &PcRing) {
    let filled = pc_ring.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for (space, pc) in pc_ring.iter() {
        let (raw, name) = match rt.space_memory(space) {
            // A unit that stepped executes in a live space; a miss is
            // runtime-state corruption and is named, not blanked.
            Err(e) => (
                format!("<space {} missing: {e}>", space.raw()),
                String::new(),
            ),
            Ok(mem) => match fetch_raw_at(mem, pc) {
                Some(w) => (
                    format!("0x{w:08x}"),
                    cellgov_ppu::decode::decode(w)
                        .ok()
                        .map(|insn| <&'static str>::from(&insn).to_string())
                        .unwrap_or_else(|| "<baddec>".into()),
                ),
                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
            },
        };
        out.push_str(&format!(
            "\n    0x{pc:08x}  raw={raw}  {name}{}",
            space_tag(space)
        ));
    }
}

pub(crate) fn append_pc_ring_terse(out: &mut String, pc_ring: &PcRing) {
    let filled = pc_ring.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for (space, pc) in pc_ring.iter() {
        out.push_str(&format!("\n    0x{pc:08x}{}", space_tag(space)));
    }
}

pub(crate) fn append_syscall_ring(out: &mut String, syscall_ring: &SyscallRing) {
    let filled = syscall_ring.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} syscalls:"));
    for (nr, pc) in syscall_ring.iter() {
        if nr >= 0x10000 {
            let idx = (nr - 0x10000) as u32;
            let name = format_hle_idx(idx);
            out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
        } else {
            out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
        }
    }
}

pub(crate) fn append_orphan_exit_info(
    diagnostic: &mut String,
    last_exit: Option<&ProcessExitInfo>,
) {
    let Some(exit) = last_exit else {
        return;
    };
    diagnostic.push_str(&format!(
        "\n  note: stale exit info captured before terminal verdict (code={}, PC=0x{:08x})",
        exit.code, exit.call_pc,
    ));
}

#[cfg(test)]
#[path = "tests/rings_tests.rs"]
mod tests;
