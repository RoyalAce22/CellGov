//! The display adapters: one instruction as assembly text, and a resolved branch target.

use core::fmt;

use crate::funcmap::FunctionMap;
use crate::instruction::PpuInstruction;

use super::render::render;

/// Renders one instruction as assembly text. `addr` is the
/// instruction's own vaddr, used to resolve relative branch targets.
pub struct AsmText<'a> {
    /// The decoded instruction to render.
    pub insn: &'a PpuInstruction,
    /// The instruction's own virtual address.
    pub addr: u64,
    /// Optional symbolizer for branch targets. `None` renders bare
    /// hex targets.
    pub symbols: Option<&'a FunctionMap>,
}

impl fmt::Display for AsmText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render(self.insn, self.addr, self.symbols, f)
    }
}

/// A resolved branch target: bare hex, plus a ` <name+0xoff>` suffix
/// when a [`FunctionMap`] resolves it (`+0x0` is omitted).
pub(super) struct Target<'a> {
    pub(super) target: u64,
    pub(super) symbols: Option<&'a FunctionMap>,
}

impl fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.target)?;
        let Some(map) = self.symbols else {
            return Ok(());
        };
        let Ok(addr32) = u32::try_from(self.target) else {
            return Ok(());
        };
        if let Some(span) = map.span_at(addr32) {
            let delta = addr32 - span.start;
            if delta == 0 {
                write!(f, " <{}>", span.display_name())?;
            } else {
                write!(f, " <{}+0x{delta:x}>", span.display_name())?;
            }
        }
        Ok(())
    }
}
