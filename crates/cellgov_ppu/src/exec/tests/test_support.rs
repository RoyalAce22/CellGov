//! Shared `execute` invocation helpers used by every per-unit
//! tests submodule. Compiled only under `cfg(test)`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;

/// Stable test-side `UnitId` so the emitted effects' `source` field is
/// predictable across tests.
pub(crate) fn uid() -> UnitId {
    UnitId::new(0)
}

/// Execute one instruction with no memory regions and no effects sink.
/// Useful for register-only / branch-only arms.
///
/// Strict: asserts that no store-buffer entries and no effects were
/// emitted. A test that accidentally drives a store- or
/// reservation-emitting instruction through this helper would
/// otherwise silently drop the side effect and pass spuriously.
pub(crate) fn exec_no_mem(insn: &PpuInstruction, s: &mut PpuState) -> ExecuteVerdict {
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(insn, s, uid(), &[], &mut effects, &mut store_buf);
    assert!(
        store_buf.is_empty(),
        "exec_no_mem: instruction staged {} store-buffer entries; \
         use exec_with_mem instead",
        store_buf.len()
    );
    assert!(
        effects.is_empty(),
        "exec_no_mem: instruction emitted {} effect(s); \
         use exec_with_mem instead",
        effects.len()
    );
    v
}

/// Execute with a single memory region view, then flush the store
/// buffer into `effects` so the caller can inspect the emitted
/// `SharedWriteIntent` packets.
pub(crate) fn exec_with_mem(
    insn: &PpuInstruction,
    s: &mut PpuState,
    base: u64,
    mem: &[u8],
    effects: &mut Vec<Effect>,
) -> ExecuteVerdict {
    let views: [(u64, &[u8]); 1] = [(base, mem)];
    let mut store_buf = StoreBuffer::new();
    let v = execute(insn, s, uid(), &views, effects, &mut store_buf);
    store_buf.flush(effects, uid());
    v
}
