//! The cache-block instructions held to the `ppu_cache_block` capture:
//! dcbst, dcbf and icbi leave the reference console's lines as they
//! were, and its touch and data-stream hints complete on an address no
//! page holds.

use crate::exec::test_support::uid;
use crate::exec::{execute, ExecuteVerdict};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

const LINES_BASE: u64 = 0x2_0000;
const PATTERN: u32 = 0x5A5A_C3C3;
/// The word indices of the two lines the probe reads back, in payload order.
const READ_BACK: [usize; 8] = [0, 1, 30, 31, 32, 33, 62, 63];

/// The probe's words, as its build emits them, with the GPRs each reads.
/// dcbst r0,r3 / dcbf r0,r6 / icbi r0,r3, with r3 at line 0 and r6 at line 1.
const PROBES: [u32; 3] = [0x7c00_186c, 0x7c00_30ac, 0x7c00_1fac];
/// dcbt / dcbtst r0,r6, then dst / dstt / dstst / dststt r6,r5 and dss / dssall.
const HINTS: [u32; 8] = [
    0x7c00_322c,
    0x7c00_31ec,
    0x7c06_2aac,
    0x7e26_2aac,
    0x7c46_2aec,
    0x7e66_2aec,
    0x7c00_066c,
    0x7e00_066c,
];

fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_cache_block/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words.len(), 2 + READ_BACK.len() + HINTS.len());
    assert_eq!(words[0], 0, "the probe ran to its end");
    words
}

#[test]
fn the_cache_block_probes_leave_the_lines_the_console_read_back() {
    let console = console_words();
    let mut page = vec![0u8; 0x1000];
    for (i, word) in page.chunks_exact_mut(4).take(64).enumerate() {
        word.copy_from_slice(&(PATTERN ^ i as u32).to_be_bytes());
    }
    let views = [cellgov_mem::RegionView::plain(LINES_BASE, &page)];
    let mut state = PpuState::new();
    state.set_gpr(3, LINES_BASE);
    state.set_gpr(6, LINES_BASE + 128);
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    for raw in PROBES {
        let insn = crate::decode::decode(raw).expect("decodes");
        let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
        assert_eq!(verdict, ExecuteVerdict::Continue, "{insn:?}");
    }
    assert!(effects.is_empty() && stores.is_empty());
    for (k, &i) in READ_BACK.iter().enumerate() {
        let at = 4 * i;
        let word = u32::from_be_bytes(page[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(word, console[2 + k], "line word {i}");
    }
}

#[test]
fn every_hint_the_console_completed_on_page_zero_continues() {
    let console = console_words();
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(LINES_BASE, &page)];
    for (n, raw) in HINTS.into_iter().enumerate() {
        assert_eq!(console[10 + n], n as u32 + 1, "the console passed hint {n}");
        let mut state = PpuState::new();
        state.set_gpr(5, 0x0020_0101);
        state.set_gpr(6, 0x100);
        let insn = crate::decode::decode(raw).expect("decodes");
        let mut effects = Vec::new();
        let mut stores = StoreBuffer::new();
        let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
        assert_eq!(verdict, ExecuteVerdict::Continue, "hint {n}: {insn:?}");
    }
}
