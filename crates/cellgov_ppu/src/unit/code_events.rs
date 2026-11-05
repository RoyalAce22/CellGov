//! The code events a traced batch records for the runtime's record of
//! unsynchronized code fetches.
//!
//! [PPC-Book2 p:11 s:1.8] after a store into an instruction location the
//! program runs `dcbst`, `sync`, `icbi`, `isync` before it runs the new
//! instruction. The unit records, in program order, each retired fetch
//! and each step of that sequence; the runtime replays them when the
//! batch commits.

use super::ppu_unit::PpuExecutionUnit;
use crate::instruction::PpuInstruction;
use cellgov_exec::CodeEvent;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CACHE_LINE_BYTES;

fn line_of(addr: u64) -> u64 {
    addr & !(PPU_CACHE_LINE_BYTES - 1)
}

/// Whether `insn` is never atomic as a whole, whatever its width.
///
/// The architecture makes none of these atomic [PPC-Book2 p:3 s:1.4]:
///
/// - a store-multiple;
/// - a string store;
/// - a cache-management store.
fn is_multiple_store(insn: &PpuInstruction) -> bool {
    matches!(
        insn,
        PpuInstruction::Stmw { .. }
            | PpuInstruction::Stswi { .. }
            | PpuInstruction::Stswx { .. }
            | PpuInstruction::Dcbz { .. }
    )
}

/// Whether a single-register store of `len` bytes at `addr` is atomic.
///
/// A word or doubleword store is atomic when its address is a multiple
/// of its width [PPC-Book2 p:3 s:1.4].
fn is_atomic_unit(addr: u64, len: u8) -> bool {
    matches!(len, 4 | 8) && addr.is_multiple_of(u64::from(len))
}

impl PpuExecutionUnit {
    /// Record the fetch at `pc` and the code events of `insn`, which
    /// retired with its stores staged from `stores_before` on.
    ///
    /// A fetch records nothing when the last event is a fetch from the
    /// same 128-byte line.
    pub(super) fn note_code_events(
        &mut self,
        insn: &PpuInstruction,
        pc: u64,
        stores_before: usize,
    ) {
        self.note_code_fetch(pc);
        let single = self.store_buf.len() == stores_before + 1 && !is_multiple_store(insn);
        for (addr, len, value) in self.store_buf.values_from(stores_before) {
            // An entry may keep register bits above its low `len` bytes.
            let value = value & 1u128.checked_shl(8 * u32::from(len)).map_or(!0, |b| b - 1);
            self.code_events.push(CodeEvent::Store {
                addr,
                len,
                value,
                atomic: single && is_atomic_unit(addr, len),
            });
        }
        let event = match *insn {
            // [PPC-Book2 p:21 s:3.2.2] `dcbf` also writes a modified block
            // to storage.
            PpuInstruction::Dcbst { ra, rb } | PpuInstruction::Dcbf { ra, rb, .. } => {
                CodeEvent::Dcbst {
                    addr: self.state.ea_x_form(ra, rb),
                }
            }
            PpuInstruction::Icbi { ra, rb } => CodeEvent::Icbi {
                addr: self.state.ea_x_form(ra, rb),
            },
            PpuInstruction::Sync { l: 0 } => CodeEvent::Sync,
            PpuInstruction::Isync => CodeEvent::Isync,
            _ => return,
        };
        self.code_events.push(event);
    }

    /// Record the fetch of an `sc` at `pc` and its context
    /// synchronization [PPC-Book3 p:4 s:1.6.1].
    pub(super) fn note_syscall_code_events(&mut self, pc: u64) {
        self.note_code_fetch(pc);
        self.code_events.push(CodeEvent::ContextSync);
    }

    fn note_code_fetch(&mut self, pc: u64) {
        let same_line = matches!(
            self.code_events.last(),
            Some(&CodeEvent::Fetch { pc: last }) if line_of(last) == line_of(pc)
        );
        if !same_line {
            self.code_events.push(CodeEvent::Fetch { pc });
        }
    }
}
