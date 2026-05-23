use cellgov_core::Runtime;

use crate::game::step_loop::{RingCursor, PC_RING_SIZE, SYSCALL_RING_SIZE};

use super::exit::ProcessExitInfo;
use super::{fetch_raw_at, format_hle_idx};

pub(in crate::game) fn append_pc_ring_with_decode(
    out: &mut String,
    rt: &Runtime,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) {
    let filled = pc_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for i in pc_cursor.iter_indices() {
        let pc = pc_ring[i];
        let (raw, name) = match fetch_raw_at(rt, pc) {
            Some(w) => (
                format!("0x{w:08x}"),
                cellgov_ppu::decode::decode(w)
                    .ok()
                    .map(|insn| <&'static str>::from(&insn).to_string())
                    .unwrap_or_else(|| "<baddec>".into()),
            ),
            None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
        };
        out.push_str(&format!("\n    0x{pc:08x}  raw={raw}  {name}"));
    }
}

pub(in crate::game) fn append_pc_ring_terse(
    out: &mut String,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) {
    let filled = pc_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for i in pc_cursor.iter_indices() {
        let pc = pc_ring[i];
        out.push_str(&format!("\n    0x{pc:08x}"));
    }
}

pub(in crate::game) fn append_syscall_ring(
    out: &mut String,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
) {
    let filled = syscall_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} syscalls:"));
    for i in syscall_cursor.iter_indices() {
        let (nr, pc) = syscall_ring[i];
        if nr >= 0x10000 {
            let idx = (nr - 0x10000) as u32;
            let name = format_hle_idx(idx);
            out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
        } else {
            out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
        }
    }
}

pub(in crate::game) fn append_orphan_exit_info(
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
mod tests {
    use super::*;

    #[test]
    fn append_orphan_exit_info_is_noop_when_none() {
        let mut s = String::from("FAULT at step 100");
        append_orphan_exit_info(&mut s, None);
        assert_eq!(s, "FAULT at step 100");
    }

    #[test]
    fn append_orphan_exit_info_appends_code_and_pc_when_some() {
        let mut s = String::from("FAULT at step 100");
        append_orphan_exit_info(
            &mut s,
            Some(&ProcessExitInfo {
                code: 0x42,
                call_pc: 0x10ab_cdef,
            }),
        );
        assert!(s.contains("code=66"), "got {s}");
        assert!(s.contains("PC=0x10abcdef"), "got {s}");
        assert!(s.contains("stale exit info"), "got {s}");
    }
}
