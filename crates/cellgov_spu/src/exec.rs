//! SPU instruction execution: translates decoded instructions into
//! `SpuState` mutations and `Effect` packets.

use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::spu_channels;
use cellgov_time::GuestTicks;

/// Outcome of executing a single SPU instruction.
#[derive(Debug)]
pub enum SpuStepOutcome {
    /// Advance PC by 4 and continue.
    Continue,
    /// PC was set by the instruction; do not advance.
    Branch,
    /// Yield to runtime with Effects.
    Yield {
        /// Effects to commit.
        effects: Vec<Effect>,
        /// Why the unit is yielding.
        reason: YieldReason,
    },
    /// Memory read the caller must service from the committed snapshot.
    ///
    /// The caller copies `size` bytes from `ea` into LS at `lsa`; when
    /// `acquire_line` is set (MFC_GETLLAR) it also emits an
    /// `Effect::ReservationAcquire` for that line.
    MemoryRead {
        /// Guest effective address to read from.
        ea: u64,
        /// Local store destination address.
        lsa: u32,
        /// Number of bytes to read.
        size: u32,
        /// Canonical line address to install a reservation for, or `None`.
        acquire_line: Option<u64>,
    },
    /// Instruction caused an architecture fault.
    Fault(SpuFault),
}

/// SPU-specific fault categories.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpuFault {
    /// LS access outside valid range.
    #[error("SPU LS access out of range at 0x{0:08x}")]
    LsOutOfRange(u32),
    /// Unsupported channel operation.
    #[error("SPU unsupported channel {} 0x{channel:02x}", if *is_write { "wrch" } else { "rdch" })]
    UnsupportedChannel {
        /// Channel number.
        channel: u8,
        /// Whether it was a read or write.
        is_write: bool,
    },
    /// Unsupported MFC command opcode.
    #[error("SPU unsupported MFC command opcode 0x{0:08x}")]
    UnsupportedMfcCommand(u32),
}

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
        // [SPU-ISA p:32 s:3. Memory-Load/Store Instructions] Load Quadword (d-form): LSA from RA + I10<<4, force low 4 bits zero.
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
        // [SPU-ISA p:33 s:3. Memory-Load/Store Instructions] Load Quadword (x-form): LSA from RA + RB, low 4 bits forced zero.
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
        // [SPU-ISA p:34 s:3. Memory-Load/Store Instructions] Load Quadword (a-form): LSA is I16<<2, ignoring registers.
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
        // [SPU-ISA p:36 s:3. Memory-Load/Store Instructions] Store Quadword (d-form): symmetric to lqd, writes register to LS.
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
        // [SPU-ISA p:37 s:3. Memory-Load/Store Instructions] Store Quadword (x-form): RA + RB indexed local-store address.
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
        // [SPU-ISA p:38 s:3. Memory-Load/Store Instructions] Store Quadword (a-form): absolute LSA from I16<<2.
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

        // [SPU-ISA p:52 s:4. Constant-Formation Instructions] Immediate Load Word: replicate sign-extended I16 into all four word slots.
        SpuInstruction::Il { rt, imm } => {
            state.set_reg_word_splat(rt, imm as i32 as u32);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:53 s:4. Constant-Formation Instructions] Immediate Load Address: replicate I18 zero-extended into all word slots.
        SpuInstruction::Ila { rt, imm } => {
            state.set_reg_word_splat(rt, imm);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:50 s:4. Constant-Formation Instructions] Immediate Load Halfword: replicate I16 into each of the eight halfword slots.
        SpuInstruction::Ilh { rt, imm } => {
            let hw = imm.to_be_bytes();
            let reg = &mut state.regs[rt as usize];
            for slot in 0..8 {
                reg[slot * 2] = hw[0];
                reg[slot * 2 + 1] = hw[1];
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:51 s:4. Constant-Formation Instructions] Immediate Load Halfword Upper: I16 placed in upper halfword of each word slot.
        SpuInstruction::Ilhu { rt, imm } => {
            state.set_reg_word_splat(rt, (imm as u32) << 16);
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:54 s:4. Constant-Formation Instructions] Immediate Or Halfword Lower: OR I16 into the lower halfword of each word slot.
        SpuInstruction::Iohl { rt, imm } => {
            for slot in 0..4 {
                let old = state.reg_word_slot(rt, slot);
                state.set_reg_word_slot(rt, slot, old | imm as u32);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:55 s:4. Constant-Formation Instructions] Form Select Mask for Bytes Immediate: each I16 bit expands to a 0x00 / 0xFF byte.
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

        // [SPU-ISA p:60 s:5. Integer and Logical Instructions] Add Word: per-slot 32-bit modulo addition.
        SpuInstruction::A { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(b));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:61 s:5. Integer and Logical Instructions] Add Word Immediate: per-slot 32-bit add of sign-extended I10.
        SpuInstruction::Ai { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a.wrapping_add(v));
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:101 s:5. Integer and Logical Instructions] And Word Immediate: per-slot AND of RA with sign-extended I10.
        SpuInstruction::Andi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a & v);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:64 s:5. Integer and Logical Instructions] Subtract from Word: per-slot RB minus RA.
        SpuInstruction::Sf { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, b.wrapping_sub(a));
            }
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:106 s:5. Integer and Logical Instructions] Or Word Immediate: per-slot OR of RA with sign-extended I10.
        SpuInstruction::Ori { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, a | v);
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:113 s:5. Integer and Logical Instructions] Nor: bitwise NOR across the full 128-bit register.
        SpuInstruction::Nor { rt, ra, rb } => {
            for i in 0..16 {
                state.regs[rt as usize][i] =
                    !(state.regs[ra as usize][i] | state.regs[rb as usize][i]);
            }
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:116 s:5. Integer and Logical Instructions] Shuffle Bytes: RC byte selectors choose from RA||RB or generate 0x00/0xFF/0x80 constants.
        SpuInstruction::Shufb { rt, ra, rb, rc } => {
            let a = state.regs[ra as usize];
            let b = state.regs[rb as usize];
            let c = state.regs[rc as usize];
            let mut result = [0u8; 16];
            for i in 0..16 {
                let sel = c[i];
                result[i] = if sel & 0xC0 == 0xC0 {
                    // Constant-generation patterns in the high bits of sel.
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
        // [SPU-ISA p:125 s:6. Shift and Rotate Instructions] Shift Left Quadword by Bytes Immediate: shift register left by I7 bytes, fill zero.
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
        // [SPU-ISA p:131 s:6. Shift and Rotate Instructions] Rotate Quadword by Bytes: byte rotate count taken from low nibble of RB preferred slot.
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

        // [SPU-ISA p:40 s:3. Memory-Load/Store Instructions] Generate Controls for Byte Insertion (d-form): build shufb mask whose target byte position holds 0x03.
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
        // [SPU-ISA p:44 s:3. Memory-Load/Store Instructions] Generate Controls for Word Insertion (d-form): build shufb mask placing 0x00..0x03 at the aligned word slot.
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

        // [SPU-ISA p:160 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word: per-slot all-ones if equal, else zero.
        SpuInstruction::Ceq { rt, ra, rb } => {
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                let b = state.reg_word_slot(rb, slot);
                state.set_reg_word_slot(rt, slot, if a == b { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }
        // [SPU-ISA p:161 s:7. Compare, Branch, and Halt Instructions] Compare Equal Word Immediate: compare RA slot to sign-extended I10.
        SpuInstruction::Ceqi { rt, ra, imm } => {
            let v = imm as i32 as u32;
            for slot in 0..4 {
                let a = state.reg_word_slot(ra, slot);
                state.set_reg_word_slot(rt, slot, if a == v { 0xFFFFFFFF } else { 0 });
            }
            SpuStepOutcome::Continue
        }

        // [SPU-ISA p:174 s:7. Compare, Branch, and Halt Instructions] Branch Relative: PC <- PC + sign-extended I16<<2, masked to LS range.
        SpuInstruction::Br { offset } => {
            state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:176 s:7. Compare, Branch, and Halt Instructions] Branch Relative and Set Link: write PC+4 link into RT then take the relative branch.
        SpuInstruction::Brsl { rt, offset } => {
            state.set_reg_word_splat(rt, state.pc + 4);
            state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
            SpuStepOutcome::Branch
        }
        // [SPU-ISA p:183 s:7. Compare, Branch, and Halt Instructions] Branch If Zero Word: branch when RT preferred slot is zero.
        SpuInstruction::Brz { rt, offset } => {
            if state.reg_word(rt) == 0 {
                state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:182 s:7. Compare, Branch, and Halt Instructions] Branch If Not Zero Word: branch when RT preferred slot is non-zero.
        SpuInstruction::Brnz { rt, offset } => {
            if state.reg_word(rt) != 0 {
                state.pc = (state.pc as i32).wrapping_add(offset << 2) as u32 & 0x3FFFC;
                SpuStepOutcome::Branch
            } else {
                SpuStepOutcome::Continue
            }
        }
        // [SPU-ISA p:178 s:7. Compare, Branch, and Halt Instructions] Branch Indirect: PC <- RA preferred slot masked to LS range.
        SpuInstruction::Bi { ra } => {
            state.pc = state.reg_word(ra) & 0x3FFFC;
            SpuStepOutcome::Branch
        }

        // [SPU-ISA p:250 s:11. Channel Instructions] Write Channel: send RT to the addressed channel.
        SpuInstruction::Wrch { channel, rt } => execute_wrch(channel, rt, state, unit_id),
        // [SPU-ISA p:248 s:11. Channel Instructions] Read Channel: capture channel value into RT, may stall on count.
        SpuInstruction::Rdch { rt, channel } => execute_rdch(rt, channel, state, unit_id),

        // [SPU-ISA p:241 s:10. Control Instructions] No Operation (Execute) is architecturally a no-op.
        // [SPU-ISA p:240 s:10. Control Instructions] No Operation (Load) consumes only an even-pipe slot.
        // [SPU-ISA p:192 s:8. Hint-for-Branch Instructions] Hint for Branch (r-form) is a hint with no architectural effect.
        // [SPU-ISA p:194 s:8. Hint-for-Branch Instructions] Hint for Branch Relative is a hint with no architectural effect.
        // [SPU-ISA p:193 s:8. Hint-for-Branch Instructions] Hint for Branch (a-form) is a hint with no architectural effect.
        // [SPU-ISA p:242 s:10. Control Instructions] Synchronize is a barrier; modeled as a no-op given in-order semantics.
        // [SPU-ISA p:150 s:7. Compare, Branch, and Halt Instructions] Halt If Equal traps when condition holds; here treated as continue.
        SpuInstruction::Nop
        | SpuInstruction::Lnop
        | SpuInstruction::Hbr
        | SpuInstruction::Hbrr
        | SpuInstruction::Hbrp
        | SpuInstruction::Sync
        | SpuInstruction::Heq => SpuStepOutcome::Continue,

        // [SPU-ISA p:238 s:10. Control Instructions] Stop and Signal halts the SPU and raises the stop signal to the PPE.
        SpuInstruction::Stop { signal: _ } => SpuStepOutcome::Yield {
            effects: vec![],
            reason: YieldReason::Finished,
        },
    }
}

fn execute_wrch(channel: u8, rt: u8, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    let val = state.reg_word(rt);
    match channel {
        // [CBE-Handbook p:453 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_LSA stores the local-store address for the MFC command being formed.
        spu_channels::MFC_LSA => {
            state.channels.mfc_lsa = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:454 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_EAH holds the high 32 bits of the 64-bit effective address.
        spu_channels::MFC_EAH => {
            state.channels.mfc_eah = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:455 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_EAL holds the low 32 bits of the effective address; alignment depends on transfer size.
        spu_channels::MFC_EAL => {
            state.channels.mfc_eal = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:455 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_Size sets the transfer size in bytes (max 16 KB).
        spu_channels::MFC_SIZE => {
            state.channels.mfc_size = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_TagID assigns a 0..31 tag value to the command being formed.
        spu_channels::MFC_TAG_ID => {
            state.channels.mfc_tag_id = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:457 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] Writing the Class ID and MFC Command Opcode enqueues the command into the SPU MFC command queue.
        spu_channels::MFC_CMD => execute_mfc_cmd(val, state, unit_id),
        // [CBE-Handbook p:458 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] MFC_WrTagMask selects the tag groups included in subsequent tag-status queries.
        spu_channels::MFC_WR_TAG_MASK => {
            state.channels.tag_mask = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:459 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] MFC_WrTagUpdate triggers when MFC_RdTagStat refreshes; immediate completion in this model.
        spu_channels::MFC_WR_TAG_UPDATE => SpuStepOutcome::Continue,
        // [CBE-Handbook p:463 s:17. SPE Channel and Related MMIO Interface sub:17.12 SPU Mailbox Channels] SPU Write Outbound Mailbox sends a 32-bit message to the PPE; values are discarded here.
        spu_channels::SPU_WR_OUT_MBOX => SpuStepOutcome::Continue,
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: true,
        }),
    }
}

fn execute_rdch(rt: u8, channel: u8, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    match channel {
        // [CBE-Handbook p:460 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] Read Tag-Group Status Channel: returns tag-status word; blocks until masked tags complete.
        spu_channels::MFC_RD_TAG_STAT => {
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
        // [CBE-Handbook p:543 s:19. DMA Transfers and Interprocessor Communication sub:19.6 Mailboxes] SPU Read Inbound Mailbox is read-blocking when the mailbox is empty.
        spu_channels::SPU_RD_IN_MBOX => {
            state.channels.pending_mbox_rt = Some(rt);
            SpuStepOutcome::Yield {
                effects: vec![Effect::MailboxReceiveAttempt {
                    mailbox: cellgov_sync::MailboxId::new(unit_id.raw()),
                    source: unit_id,
                }],
                reason: YieldReason::MailboxAccess,
            }
        }
        // [CBE-Handbook p:462 s:17. SPE Channel and Related MMIO Interface sub:17.11 MFC Read Atomic Command Status Channel] Reports success/failure status for the most recent atomic command (e.g. putllc).
        spu_channels::MFC_RD_ATOMIC_STAT => {
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
        // [CBEA p:61 s:7. MFC Commands sub:7.6 Put Commands (Local Storage to Main Storage)] put: copy LS bytes to main storage.
        spu_channels::MFC_PUT => {
            let lsa_usize = lsa as usize;
            let size_usize = size as usize;
            let ls_bytes = state.ls[lsa_usize..lsa_usize + size_usize].to_vec();

            let src =
                ByteRange::new(GuestAddr::new(lsa as u64), size as u64).expect("valid LS range");
            let dst = ByteRange::new(GuestAddr::new(ea), size as u64).expect("valid EA range");
            let request = DmaRequest::new(DmaDirection::Put, src, dst, unit_id)
                .expect("matching sizes")
                .with_tag_id(state.channels.mfc_tag_id as u8);
            // [CBEA p:65 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] Self-store overlapping the reserved line clears the reservation.
            if let Some(line) = state.reservation {
                if line.overlaps_range(ea, size as u64) {
                    state.reservation = None;
                }
            }
            SpuStepOutcome::Yield {
                effects: vec![Effect::DmaEnqueue {
                    request,
                    payload: Some(ls_bytes),
                }],
                reason: YieldReason::DmaSubmitted,
            }
        }
        // [CBEA p:60 s:7. MFC Commands sub:7.5 Get Commands (Main Storage to Local Storage)] get: copy main-storage bytes into LS.
        spu_channels::MFC_GET => {
            state.channels.pending_get = Some((ea, lsa, size, state.channels.mfc_tag_id as u8));
            SpuStepOutcome::Yield {
                effects: vec![],
                reason: YieldReason::DmaSubmitted,
            }
        }
        // [CBEA p:65 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] getllar: load 128B cache line and acquire reservation on it.
        spu_channels::MFC_GETLLAR => {
            state.channels.atomic_status = 0;
            let line = cellgov_sync::ReservedLine::containing(ea);
            state.reservation = Some(line);
            SpuStepOutcome::MemoryRead {
                ea,
                lsa,
                size: 128,
                acquire_line: Some(line.addr()),
            }
        }
        // [CBEA p:66 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] putllc: conditional store that succeeds only if the local reservation is still held for this line.
        spu_channels::MFC_PUTLLC => {
            let line = cellgov_sync::ReservedLine::containing(ea);
            let success = match state.reservation {
                Some(l) => l.addr() == line.addr(),
                None => false,
            };
            state.reservation = None;
            if success {
                let lsa_usize = lsa as usize;
                let ls_bytes = state.ls[lsa_usize..lsa_usize + 128].to_vec();
                let range = ByteRange::new(GuestAddr::new(ea), 128).expect("valid EA range");
                state.channels.atomic_status = 0;
                SpuStepOutcome::Yield {
                    effects: vec![Effect::ConditionalStore {
                        range,
                        bytes: WritePayload::new(ls_bytes),
                        ordering: PriorityClass::Normal,
                        source: unit_id,
                        source_time: GuestTicks::ZERO,
                    }],
                    reason: YieldReason::DmaSubmitted,
                }
            } else {
                state.channels.atomic_status = 1;
                SpuStepOutcome::Continue
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
        assert_eq!(s.reg_word(0), 0x104);
        assert_eq!(s.pc, 0x140);
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
        assert_eq!(s.regs[5][0], 0x00);
        assert_eq!(s.regs[5][1], 0x01);
        assert_eq!(s.regs[5][2], 0x02);
        assert_eq!(s.regs[5][3], 0x03);
        assert_eq!(s.regs[5][4], 0x14);
    }

    #[test]
    fn lqa_stqa_roundtrip() {
        let mut s = SpuState::new();
        s.set_reg_word_splat(5, 0xCAFEBABE);
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
        execute(&SpuInstruction::Fsmbi { rt: 3, imm: 0xFFFF }, &mut s, uid());
        assert!(s.regs[3].iter().all(|&b| b == 0xFF));
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
