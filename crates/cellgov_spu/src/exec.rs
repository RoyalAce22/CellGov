//! SPU instruction execution.
//!
//! Takes a decoded `SpuInstruction` and an `SpuState`, applies the
//! instruction's semantics (register/LS mutation), and returns an
//! `SpuStepOutcome` indicating whether execution should continue,
//! yield with Effects, or fault.
//!
//! This is the only layer that translates decoded instructions into
//! local state changes and `Effect` packets. Decode does not know
//! about state; state does not know about Effects.

use crate::channels;
use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr};

/// What happened after executing one instruction.
#[derive(Debug)]
pub enum SpuStepOutcome {
    /// Instruction executed, advance PC by 4, keep running.
    Continue,
    /// PC was set explicitly (branch taken). Do not advance.
    Branch,
    /// Instruction requires runtime mediation.
    Yield {
        /// Effects to commit.
        effects: Vec<Effect>,
        /// Why the unit is yielding.
        reason: YieldReason,
    },
    /// Read from guest memory into local store. The execute layer
    /// does not hold the `ExecutionContext`; `run_until_yield`
    /// fulfills the read from the frozen committed snapshot exposed
    /// by the context.
    MemoryRead {
        /// Guest effective address to read from.
        ea: u64,
        /// Local store destination address.
        lsa: u32,
        /// Number of bytes to read.
        size: u32,
    },
    /// Instruction caused an architecture fault.
    Fault(SpuFault),
}

/// SPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpuFault {
    /// LS access outside valid range.
    LsOutOfRange(u32),
    /// Unsupported channel operation.
    UnsupportedChannel {
        /// Channel number.
        channel: u8,
        /// Whether it was a read or write.
        is_write: bool,
    },
    /// Unsupported MFC command opcode.
    UnsupportedMfcCommand(u32),
}

// -- Helper: compute LS address and validate --

fn ls_addr(raw: u32, ls_len: usize) -> Result<usize, SpuFault> {
    let a = (raw & 0x3FFF0) as usize;
    if a + 16 > ls_len {
        Err(SpuFault::LsOutOfRange(raw))
    } else {
        Ok(a)
    }
}

/// Execute a single decoded SPU instruction.
pub fn execute(insn: &SpuInstruction, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    match *insn {
        // =================================================================
        // Loads / stores
        // =================================================================
        SpuInstruction::Lqd { rt, ra, imm } => {
            let raw = state.reg_word(ra).wrapping_add((imm as i32 as u32) << 4);
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.regs[rt as usize].copy_from_slice(&state.ls[a..a + 16]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }
        SpuInstruction::Lqx { rt, ra, rb } => {
            let raw = state.reg_word(ra).wrapping_add(state.reg_word(rb));
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.regs[rt as usize].copy_from_slice(&state.ls[a..a + 16]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }
        SpuInstruction::Lqa { rt, imm } => {
            let raw = (imm as i32 as u32) << 2;
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.regs[rt as usize].copy_from_slice(&state.ls[a..a + 16]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }
        SpuInstruction::Stqd { rt, ra, imm } => {
            let raw = state.reg_word(ra).wrapping_add((imm as i32 as u32) << 4);
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.ls[a..a + 16].copy_from_slice(&state.regs[rt as usize]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }
        SpuInstruction::Stqx { rt, ra, rb } => {
            let raw = state.reg_word(ra).wrapping_add(state.reg_word(rb));
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.ls[a..a + 16].copy_from_slice(&state.regs[rt as usize]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }
        SpuInstruction::Stqa { rt, imm } => {
            let raw = (imm as i32 as u32) << 2;
            match ls_addr(raw, state.ls.len()) {
                Ok(a) => {
                    state.ls[a..a + 16].copy_from_slice(&state.regs[rt as usize]);
                    SpuStepOutcome::Continue
                }
                Err(f) => SpuStepOutcome::Fault(f),
            }
        }

        // =================================================================
        // Constant formation
        // =================================================================
        SpuInstruction::Il { rt, imm } => {
            state.set_reg_word_splat(rt, imm as i32 as u32);
            SpuStepOutcome::Continue
        }
        SpuInstruction::Ila { rt, imm } => {
            state.set_reg_word_splat(rt, imm);
            SpuStepOutcome::Continue
        }
        SpuInstruction::Ilh { rt, imm } => {
            let hw = imm.to_be_bytes();
            let reg = &mut state.regs[rt as usize];
            for slot in 0..8 {
                reg[slot * 2] = hw[0];
                reg[slot * 2 + 1] = hw[1];
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Ilhu { rt, imm } => {
            state.set_reg_word_splat(rt, (imm as u32) << 16);
            SpuStepOutcome::Continue
        }
        SpuInstruction::Iohl { rt, imm } => {
            for slot in 0..4 {
                let old = state.reg_word_slot(rt, slot);
                state.set_reg_word_slot(rt, slot, old | imm as u32);
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Fsmbi { rt, imm } => {
            let mut result = [0u8; 16];
            for (i, byte) in result.iter_mut().enumerate() {
                *byte = if (imm >> (15 - i)) & 1 != 0 {
                    0xFF
                } else {
                    0x00
                };
            }
            state.regs[rt as usize] = result;
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Integer arithmetic
        // =================================================================
        SpuInstruction::A { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(b));
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Ai { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(v));
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Andi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a & v);
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Sf { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, b.wrapping_sub(a));
            }
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Logical
        // =================================================================
        SpuInstruction::Ori { rt, ra, imm } => {
            // ori is RI10 with sign-extended 10-bit immediate, applied per-word
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a | v);
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Nor { rt, ra, rb } => {
            for i in 0..16 {
                state.regs[rt as usize][i] =
                    !(state.regs[ra as usize][i] | state.regs[rb as usize][i]);
            }
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Shuffle / shift / rotate
        // =================================================================
        SpuInstruction::Shufb { rt, ra, rb, rc } => {
            let a = state.regs[ra as usize];
            let b = state.regs[rb as usize];
            let c = state.regs[rc as usize];
            let mut result = [0u8; 16];
            for i in 0..16 {
                let sel = c[i];
                result[i] = if sel & 0xC0 == 0xC0 {
                    // Special pattern
                    if sel & 0xE0 == 0xC0 {
                        0x00
                    } else if sel & 0xE0 == 0xE0 {
                        0xFF
                    } else {
                        0x80
                    }
                } else if sel & 0x10 == 0 {
                    a[(sel & 0xF) as usize]
                } else {
                    b[(sel & 0xF) as usize]
                };
            }
            state.regs[rt as usize] = result;
            SpuStepOutcome::Continue
        }
        SpuInstruction::Shlqbyi { rt, ra, imm } => {
            let shift = (imm & 0x1F) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for i in 0..16 {
                dst[i] = if i + shift < 16 { src[i + shift] } else { 0 };
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }
        SpuInstruction::Rotqby { rt, ra, rb } => {
            let shift = (state.reg_word(rb) & 0xF) as usize;
            let src = state.regs[ra as usize];
            let mut dst = [0u8; 16];
            for i in 0..16 {
                dst[i] = src[(i + shift) & 0xF];
            }
            state.regs[rt as usize] = dst;
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Generate controls
        // =================================================================
        SpuInstruction::Cbd { rt, ra, imm } => {
            let addr = state.reg_word(ra).wrapping_add(imm as u32);
            let pos = (addr & 0xF) as usize;
            let mut mask = [0u8; 16];
            for (i, byte) in mask.iter_mut().enumerate() {
                *byte = if i == pos { 0x03 } else { 0x10 + i as u8 };
            }
            state.regs[rt as usize] = mask;
            SpuStepOutcome::Continue
        }
        SpuInstruction::Cwd { rt, ra, imm } => {
            let addr = state.reg_word(ra).wrapping_add(imm as u32);
            let pos = (addr & 0xC) as usize;
            let mut mask = [0u8; 16];
            for (i, byte) in mask.iter_mut().enumerate() {
                *byte = if i >= pos && i < pos + 4 {
                    (i - pos) as u8
                } else {
                    0x10 + i as u8
                };
            }
            state.regs[rt as usize] = mask;
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Compare
        // =================================================================
        SpuInstruction::Ceq { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, if a == b { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }
        SpuInstruction::Ceqi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, if a == v { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }

        // =================================================================
        // Branch
        // =================================================================
        SpuInstruction::Br { offset } => {
            state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
            SpuStepOutcome::Branch
        }
        SpuInstruction::Brsl { rt, offset } => {
            state.set_reg_word_splat(rt, state.pc + 4);
            state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
            SpuStepOutcome::Branch
        }
        SpuInstruction::Brz { rt, offset } => {
            if state.reg_word(rt) == 0 {
                state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        SpuInstruction::Brnz { rt, offset } => {
            if state.reg_word(rt) != 0 {
                state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        SpuInstruction::Bi { ra } => {
            state.pc = state.reg_word(ra) & 0x3FFFC;
            SpuStepOutcome::Branch
        }

        // =================================================================
        // Channel operations
        // =================================================================
        SpuInstruction::Wrch { channel, rt } => execute_wrch(channel, rt, state, unit_id),
        SpuInstruction::Rdch { rt, channel } => execute_rdch(rt, channel, state, unit_id),

        // =================================================================
        // Hint / nop / sync / control
        // =================================================================
        SpuInstruction::Nop
        | SpuInstruction::Lnop
        | SpuInstruction::Hbr
        | SpuInstruction::Hbrr
        | SpuInstruction::Hbrp
        | SpuInstruction::Sync
        | SpuInstruction::Heq => SpuStepOutcome::Continue,

        SpuInstruction::Stop { signal: _ } => SpuStepOutcome::Yield {
            effects: vec![],
            reason: YieldReason::Finished,
        },
    }
}

// -- Channel helpers (unchanged from previous implementation) --

fn execute_wrch(channel: u8, rt: u8, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    let val = state.reg_word(rt);
    match channel {
        channels::MFC_LSA => {
            state.channels.mfc_lsa = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_EAH => {
            state.channels.mfc_eah = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_EAL => {
            state.channels.mfc_eal = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_SIZE => {
            state.channels.mfc_size = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_TAG_ID => {
            state.channels.mfc_tag_id = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_CMD => execute_mfc_cmd(val, state, unit_id),
        channels::MFC_WR_TAG_MASK => {
            state.channels.tag_mask = val;
            SpuStepOutcome::Continue
        }
        channels::MFC_WR_TAG_UPDATE => {
            // Tag status update request. In our simplified model, tag
            // completion is immediate, so this is a no-op.
            SpuStepOutcome::Continue
        }
        channels::SPU_WR_OUT_MBOX => {
            // Outbound mailbox write (SPU -> PPU). In the interpreter
            // we treat this as a no-op; the value is discarded.
            SpuStepOutcome::Continue
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: true,
        }),
    }
}

fn execute_rdch(rt: u8, channel: u8, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    match channel {
        channels::MFC_RD_TAG_STAT => {
            let masked = state.channels.tag_status & state.channels.tag_mask;
            if masked == state.channels.tag_mask {
                state.set_reg_word_splat(rt, state.channels.tag_status);
                SpuStepOutcome::Continue
            } else {
                SpuStepOutcome::Yield {
                    effects: vec![],
                    reason: YieldReason::DmaWait,
                }
            }
        }
        channels::SPU_RD_IN_MBOX => {
            state.channels.pending_mbox_rt = Some(rt);
            SpuStepOutcome::Yield {
                effects: vec![Effect::MailboxReceiveAttempt {
                    mailbox: cellgov_sync::MailboxId::new(unit_id.raw()),
                    source: unit_id,
                }],
                reason: YieldReason::MailboxAccess,
            }
        }
        channels::MFC_RD_ATOMIC_STAT => {
            state.set_reg_word_splat(rt, state.channels.atomic_status);
            SpuStepOutcome::Continue
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: false,
        }),
    }
}

fn execute_mfc_cmd(cmd: u32, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    let ea = ((state.channels.mfc_eah as u64) << 32) | state.channels.mfc_eal as u64;
    let lsa = state.channels.mfc_lsa;
    let size = state.channels.mfc_size;

    match cmd {
        channels::MFC_PUT => {
            let lsa_usize = lsa as usize;
            let size_usize = size as usize;
            let ls_bytes = state.ls[lsa_usize..lsa_usize + size_usize].to_vec();

            let src =
                ByteRange::new(GuestAddr::new(lsa as u64), size as u64).expect("valid LS range");
            let dst = ByteRange::new(GuestAddr::new(ea), size as u64).expect("valid EA range");
            let request =
                DmaRequest::new(DmaDirection::Put, src, dst, unit_id).expect("matching sizes");
            state.channels.tag_status |= 1 << state.channels.mfc_tag_id;
            SpuStepOutcome::Yield {
                effects: vec![Effect::DmaEnqueue {
                    request,
                    payload: Some(ls_bytes),
                }],
                reason: YieldReason::DmaSubmitted,
            }
        }
        channels::MFC_GET => {
            state.channels.tag_status |= 1 << state.channels.mfc_tag_id;
            state.channels.pending_get = Some((ea, lsa, size));
            SpuStepOutcome::Yield {
                effects: vec![],
                reason: YieldReason::DmaSubmitted,
            }
        }
        channels::MFC_GETLLAR => {
            state.channels.atomic_status = 0; // reservation acquired
            SpuStepOutcome::MemoryRead { ea, lsa, size: 128 }
        }
        channels::MFC_PUTLLC => {
            let lsa_usize = lsa as usize;
            let ls_bytes = state.ls[lsa_usize..lsa_usize + 128].to_vec();

            let src = ByteRange::new(GuestAddr::new(lsa as u64), 128).expect("valid LS range");
            let dst = ByteRange::new(GuestAddr::new(ea), 128).expect("valid EA range");
            let request =
                DmaRequest::new(DmaDirection::Put, src, dst, unit_id).expect("matching sizes");
            state.channels.atomic_status = 0; // conditional store succeeded
            SpuStepOutcome::Yield {
                effects: vec![Effect::DmaEnqueue {
                    request,
                    payload: Some(ls_bytes),
                }],
                reason: YieldReason::DmaSubmitted,
            }
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedMfcCommand(cmd)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SpuState;

    fn uid() -> UnitId {
        UnitId::new(0)
    }

    #[test]
    fn il_splats_all_slots() {
        let mut s = SpuState::new();
        execute(&SpuInstruction::Il { rt: 5, imm: -1 }, &mut s, uid());
        for slot in 0..4 {
            assert_eq!(s.reg_word_slot(5, slot), 0xFFFFFFFF);
        }
    }

    #[test]
    fn ilhu_shifts_left_16() {
        let mut s = SpuState::new();
        execute(&SpuInstruction::Ilhu { rt: 3, imm: 0x1337 }, &mut s, uid());
        assert_eq!(s.reg_word(3), 0x13370000);
    }

    #[test]
    fn iohl_ors_lower_halfword() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(3, 0x13370000);
        execute(&SpuInstruction::Iohl { rt: 3, imm: 0xBAAD }, &mut s, uid());
        assert_eq!(s.reg_word(3), 0x1337BAAD);
    }

    #[test]
    fn sf_subtracts_ra_from_rb() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(1, 3);
        s.set_reg_word_splat(2, 10);
        execute(
            &SpuInstruction::Sf {
                rt: 3,
                ra: 1,
                rb: 2,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.reg_word(3), 7);
    }

    #[test]
    fn ori_with_zero_is_move() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(1, 0xDEADBEEF);
        execute(
            &SpuInstruction::Ori {
                rt: 2,
                ra: 1,
                imm: 0,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.reg_word(2), 0xDEADBEEF);
    }

    #[test]
    fn shufb_identity_pattern() {
        let mut s = SpuState::new();
        s.regs[1] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, 0x1F,
        ];
        // Identity pattern: bytes 0..15
        s.regs[3] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        execute(
            &SpuInstruction::Shufb {
                rt: 4,
                ra: 1,
                rb: 2,
                rc: 3,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.regs[4], s.regs[1]);
    }

    #[test]
    fn bi_branches_to_register() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(0, 0x3A0);
        let outcome = execute(&SpuInstruction::Bi { ra: 0 }, &mut s, uid());
        assert!(matches!(outcome, SpuStepOutcome::Branch));
        assert_eq!(s.pc, 0x3A0);
    }

    #[test]
    fn brsl_sets_link_and_branches() {
        let mut s = SpuState::new();
        s.pc = 0x100;
        let outcome = execute(&SpuInstruction::Brsl { rt: 0, offset: 16 }, &mut s, uid());
        assert!(matches!(outcome, SpuStepOutcome::Branch));
        assert_eq!(s.reg_word(0), 0x104); // link = PC + 4
        assert_eq!(s.pc, 0x140); // 0x100 + 16*4
    }

    #[test]
    fn cwd_generates_word_insertion_mask() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(3, 0x3000);
        execute(
            &SpuInstruction::Cwd {
                rt: 5,
                ra: 3,
                imm: 0,
            },
            &mut s,
            uid(),
        );
        // addr & 0xC = 0, so word insertion at bytes 0-3
        assert_eq!(s.regs[5][0], 0x00);
        assert_eq!(s.regs[5][1], 0x01);
        assert_eq!(s.regs[5][2], 0x02);
        assert_eq!(s.regs[5][3], 0x03);
        assert_eq!(s.regs[5][4], 0x14); // 0x10 + 4
    }

    #[test]
    fn lqa_stqa_roundtrip() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(5, 0xCAFEBABE);
        // stqa at address based on imm (imm << 2, masked)
        let addr_imm: i16 = (0x3000u16 >> 2) as i16;
        execute(
            &SpuInstruction::Stqa {
                rt: 5,
                imm: addr_imm,
            },
            &mut s,
            uid(),
        );
        execute(
            &SpuInstruction::Lqa {
                rt: 6,
                imm: addr_imm,
            },
            &mut s,
            uid(),
        );
        assert_eq!(s.regs[6], s.regs[5]);
    }

    #[test]
    fn fsmbi_creates_byte_mask() {
        let mut s = SpuState::new();
        // imm = 0xFFFF -> all bytes = 0xFF
        execute(&SpuInstruction::Fsmbi { rt: 3, imm: 0xFFFF }, &mut s, uid());
        assert!(s.regs[3].iter().all(|&b| b == 0xFF));
        // imm = 0x0000 -> all bytes = 0x00
        execute(&SpuInstruction::Fsmbi { rt: 4, imm: 0x0000 }, &mut s, uid());
        assert!(s.regs[4].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn stop_yields_finished() {
        let mut s = SpuState::new();
        let outcome = execute(&SpuInstruction::Stop { signal: 0 }, &mut s, uid());
        assert!(matches!(
            outcome,
            SpuStepOutcome::Yield {
                reason: YieldReason::Finished,
                ..
            }
        ));
    }

    #[test]
    fn nop_continues() {
        let mut s = SpuState::new();
        assert!(matches!(
            execute(&SpuInstruction::Nop, &mut s, uid()),
            SpuStepOutcome::Continue
        ));
        assert!(matches!(
            execute(&SpuInstruction::Lnop, &mut s, uid()),
            SpuStepOutcome::Continue
        ));
    }
}
