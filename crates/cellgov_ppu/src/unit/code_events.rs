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
        for (addr, len, word) in self.store_buf.words_from(stores_before) {
            let word = word.filter(|_| addr % 4 == 0);
            self.code_events.push(CodeEvent::Store { addr, len, word });
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
