//! The fetch and the decode of the instruction a batch runs next:
//! - the fetchable span;
//! - the shadow;
//! - the word stored in a region;
//! - the check of a fetch against the buffered stores;
//! - the split of a fused head.

use super::run::Cursor;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::unit::ppu_unit::PpuExecutionUnit;
use crate::{decode, instruction};
use cellgov_exec::ExecutionContext;

impl PpuExecutionUnit {
    /// The instruction at `step_pc`.
    ///
    /// When `fetchable` does not hold the word, the unit fetches the span
    /// that holds it, and `fetchable` and `code` move to that span. A
    /// translating thread decodes the stored word. Another thread reads
    /// the shadow first.
    ///
    /// # Errors
    ///
    /// Returns the fault of the fetch or of the decode.
    #[inline(always)]
    pub(super) fn fetch_step<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        step_pc: u64,
        translating: bool,
        fetchable: &mut (u64, u64),
        code: &mut CodeBytes<'m>,
    ) -> Result<instruction::PpuInstruction, PpuFault> {
        if !holds_word(*fetchable, step_pc) {
            let window = if translating {
                self.translated_fetch(mem, step_pc)
            } else {
                fetch_window(mem, step_pc)
            };
            let (span, bytes) = window?;
            *fetchable = span;
            *code = bytes;
        }
        if translating {
            decode_stored(step_pc, *code).map(|(_, insn)| insn)
        } else {
            self.fetch_decode(step_pc, *code)
        }
    }

    /// The instruction at `step_pc`: the shadow's copy, or a fresh decode
    /// that refreshes the shadow.
    #[inline]
    fn fetch_decode(
        &mut self,
        step_pc: u64,
        code: CodeBytes<'_>,
    ) -> Result<instruction::PpuInstruction, PpuFault> {
        if let Some(cached) = self
            .instruction_shadow
            .as_ref()
            .and_then(|s| s.get(step_pc))
        {
            self.shadow_hits += 1;
            return Ok(cached);
        }
        self.shadow_misses += 1;
        let (raw, insn) = decode_stored(step_pc, code)?;
        if let Some(s) = self.instruction_shadow.as_mut() {
            let _ = s.refresh(step_pc, raw);
        }
        Ok(insn)
    }

    /// Whether the fetch at `pc` reads a word a buffered store covers.
    ///
    /// `span` bounds the stores, so a fetch outside it costs two compares.
    /// The checked range is 8 bytes, both slots a fused head reads.
    #[inline]
    pub(super) fn fetches_buffered_store(&self, (lo, hi): (u64, u64), pc: u64) -> bool {
        let end = pc.saturating_add(8);
        pc < hi && end > lo && self.store_buf.has_store_in_range(pc, end)
    }

    /// The instruction the loop runs at `step_pc`: `insn` or the stored word.
    ///
    /// The loop runs the stored word for a `Consumed` slot and for a
    /// fused head that splits.
    ///
    /// # Errors
    ///
    /// Returns the fault of the stored word's decode.
    #[inline(always)]
    pub(super) fn split_or_fused(
        &self,
        ctx: &ExecutionContext<'_>,
        cur: &Cursor<'_>,
        insn: instruction::PpuInstruction,
        step_pc: u64,
        forced: bool,
    ) -> Result<instruction::PpuInstruction, PpuFault> {
        // A fused head retires both of its slots. The loop reaches a
        // `Consumed` slot only by a branch to it or after a split head.
        // The instruction stored there then runs [PPC-Book1 p:17 s:2.2].
        // A head splits and runs as its first instruction alone when:
        // - the budget cannot retire both slots,
        // - a break is set on the second slot,
        // - per-step tracing is on,
        // - a full-state window is set,
        // - the second slot is not fetchable,
        // - a Raw SPU window is mapped and the second half reads memory
        //   after the first half accessed it, or
        // - a PPE sibling is runnable.
        // The trace and the full-state window record the state after each
        // instruction, and a fused head has no state between its slots.
        // A Raw SPU window changes only between batches, and the batch
        // ends at the first half that reaches it. Beside a sibling the
        // batch ends at the first instruction that reaches a grid line
        // (`leads_sibling`), and that can be the first half.
        // A head whose second half refused fused also runs split, so
        // its first instruction retires before the refusal.
        let split = match insn {
            instruction::PpuInstruction::Consumed => true,
            _ if insn.is_super_pair() => {
                forced
                    || cur.remaining < 2
                    || !holds_word(cur.fetchable, step_pc.wrapping_add(4))
                    || self.break_pc == Some(step_pc + 4)
                    || ctx.trace_per_step()
                    || self.full_state_window.is_some()
                    || (cur.window_span.is_some() && insn.reads_after_access())
                    || ctx.ppe_siblings() > 0
            }
            _ => false,
        };
        if !split {
            return Ok(insn);
        }
        decode_stored(step_pc, cur.code).map(|(_, insn)| insn)
    }
}

/// Whether the word at `pc` lies in the window `(start, last)`, where
/// `last` is the last word that starts in the span.
#[inline]
fn holds_word((start, last): (u64, u64), pc: u64) -> bool {
    start <= pc && pc <= last
}

/// The base guest address of a region and its bytes.
pub(in crate::unit) type CodeBytes<'a> = (u64, &'a [u8]);

/// The fetchable span that holds the word at `pc`, and its region.
///
/// The span is its first byte and the last word that starts in it.
///
/// # Errors
///
/// Returns `PpuInterrupt::InstructionStorage` if a byte of the word is
/// not fetchable. The interrupt sets `no_execute` if that byte is mapped.
pub(in crate::unit) fn fetch_window(
    mem: &cellgov_mem::GuestMemory,
    pc: u64,
) -> Result<((u64, u64), CodeBytes<'_>), PpuFault> {
    use cellgov_mem::FetchRefusal;
    let refused = |r: FetchRefusal| {
        PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: r == FetchRefusal::NoExecute,
            protection: false,
        })
    };
    let code = mem.fetch_code(pc).map_err(refused)?;
    let (start, end) = code.span;
    match pc.checked_add(4) {
        Some(word_end) if word_end <= end => Ok(((start, end - 4), (code.base, code.bytes))),
        _ => Err(refused(
            mem.fetch_span(end).err().unwrap_or(FetchRefusal::Unmapped),
        )),
    }
}

/// The word stored at `pc` in a region, and its decode, without the shadow.
///
/// # Errors
///
/// - A fetch outside the region: the instruction storage interrupt.
/// - A word with no decode: the illegal-instruction program interrupt.
#[inline]
fn decode_stored(
    pc: u64,
    (base, mem): CodeBytes<'_>,
) -> Result<(u32, instruction::PpuInstruction), PpuFault> {
    let bytes = pc
        .checked_sub(base)
        .and_then(|off| usize::try_from(off).ok())
        .and_then(|pc| mem.get(pc..pc.checked_add(4)?))
        .ok_or(PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: false,
            protection: false,
        }))?;
    let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let insn = decode::decode(raw).map_err(|_| PpuFault::unrecognized_word())?;
    Ok((raw, insn))
}
