//! An instruction that faults leaves the PPU state as it found it. A
//! batch can then commit the instructions before it and no part of the
//! one that faults.

use crate::exec::test_support::uid;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::fuzz::generation_descriptors;
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Register values that send an access to one of:
/// - the bytes past the end of the one mapped page
/// - an unmapped page
/// - a misaligned address
const SEEDS: [u64; 6] = [0xFF0, 0xFF8, 0xFFC, 0xFE1, 0x1001, 0xFFFF_0000];

#[test]
fn a_faulting_instruction_changes_no_architectural_state() {
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let mut faults = 0usize;
    let mut changed = Vec::new();
    for descriptor in generation_descriptors() {
        let mut words = vec![descriptor.canonical_word];
        words.extend(descriptor.immediate_boundary_words());
        words.extend((0..32).filter_map(|v| descriptor.alias_word(v)));
        for word in words {
            let Ok(insn) = crate::decode::decode(word) else {
                continue;
            };
            for seed in SEEDS {
                let mut state = PpuState::new();
                state.set_gpr_all([seed; 32]);
                state.set_xer(32);
                state.vrsave_written = true;
                let before = PpuArchitecturalState::capture(&state);
                let hash = state.state_hash();
                let mut effects = Vec::new();
                let mut stores = StoreBuffer::new();
                let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
                if !matches!(
                    verdict,
                    ExecuteVerdict::Fault(_) | ExecuteVerdict::MemFault(_)
                ) {
                    continue;
                }
                faults += 1;
                if PpuArchitecturalState::capture(&state) != before || state.state_hash() != hash {
                    changed.push(format!("{insn:?} seed 0x{seed:x}"));
                }
            }
        }
    }
    assert!(faults > 100, "only {faults} faulting cases ran");
    assert!(changed.is_empty(), "{changed:#?}");
}
