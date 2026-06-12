//! Arithmetic, logical, shift, rotate, compare, CR/SPR-move dispatch.
//! Every arm here is a pure register-to-register or register-to-CR
//! operation; nothing in this module touches memory or emits effects.

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;

pub(crate) fn execute(insn: &PpuInstruction, state: &mut PpuState) -> ExecuteVerdict {
    match *insn {
        // Integer arithmetic / logical
        // [PPC-Book1 p:51 s:3.3.8] addi: RT <- (RA|0) + EXTS(SI); RA=0 selects literal zero, else GPR(RA).
        PpuInstruction::Addi { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add(imm as i64 as u64);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:51 s:3.3.8] addis: RT <- (RA|0) + (SI << 16); D-form add immediate shifted.
        PpuInstruction::Addis { rt, ra, imm } => {
            let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
            state.gpr[rt as usize] = base.wrapping_add((imm as i64 as u64) << 16);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:53 s:3.3.8] subfic: RT <- ~(RA) + EXTS(SI) + 1; sets CA from carry.
        PpuInstruction::Subfic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            let (result, borrow) = b.overflowing_sub(a);
            state.gpr[rt as usize] = result;
            state.set_xer_ca(!borrow);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:56 s:3.3.8] mulli: signed (RA) * EXTS(SI), low 64 bits placed in RT.
        PpuInstruction::Mulli { rt, ra, imm } => {
            let a = state.gpr[ra as usize] as i64;
            let b = imm as i64;
            state.gpr[rt as usize] = a.wrapping_mul(b) as u64;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:51 s:3.3.8] addic: RT <- (RA) + EXTS(SI); always sets CA from carry-out.
        PpuInstruction::Addic { rt, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            let (result, carry) = a.overflowing_add(b);
            state.gpr[rt as usize] = result;
            state.set_xer_ca(carry);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:51 s:3.3.8] addic.: addic with implicit Rc=1 updating CR0 from result.
        PpuInstruction::AddicDot { rt, ra, imm } => {
            // Same arithmetic as Addic; ISA exposes the dot form as
            // primary 13 with implicit Rc=1 (CR0 always updated).
            let a = state.gpr[ra as usize];
            let b = imm as i64 as u64;
            let (result, carry) = a.overflowing_add(b);
            state.gpr[rt as usize] = result;
            state.set_xer_ca(carry);
            state.set_cr0_from_result(result);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:52 s:3.3.8] add: RT <- (RA) + (RB); OE sets SO/OV, Rc updates CR0.
        PpuInstruction::Add { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let result = a.wrapping_add(b);
            state.gpr[rt as usize] = result;
            if oe {
                let ov = ((a ^ result) & (b ^ result)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:52 s:3.3.8] subf: RT <- ~(RA) + (RB) + 1, i.e. (RB) - (RA).
        PpuInstruction::Subf { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let result = b.wrapping_sub(a);
            state.gpr[rt as usize] = result;
            if oe {
                let ov = ((b ^ a) & (b ^ result)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:53 s:3.3.8] subfc: subtract from carrying; sets CA from borrow-out.
        PpuInstruction::Subfc { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let (result, borrow) = b.overflowing_sub(a);
            state.gpr[rt as usize] = result;
            state.set_xer_ca(!borrow);
            if oe {
                let ov = ((b ^ a) & (b ^ result)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:54 s:3.3.8] subfe: ~(RA) + (RB) + CA; carry-in from XER[CA].
        PpuInstruction::Subfe { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (s1, c1) = b.overflowing_add(!a);
            let (s2, c2) = s1.overflowing_add(ca_in);
            state.gpr[rt as usize] = s2;
            state.set_xer_ca(c1 || c2);
            if oe {
                let ov = ((b ^ a) & (b ^ s2)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(s2);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:55 s:3.3.8] neg: RT <- ~(RA) + 1; OV set if RA is the most-negative value.
        PpuInstruction::Neg { rt, ra, oe, rc } => {
            let a = state.gpr[ra as usize];
            let result = (a as i64).wrapping_neg() as u64;
            state.gpr[rt as usize] = result;
            if oe {
                state.set_xer_ov(a == 0x8000_0000_0000_0000);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:56 s:3.3.8] mullw: signed 32x32 product, low 32 bits sign-extended into RT.
        PpuInstruction::Mullw { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize] as i32 as i64;
            let b = state.gpr[rb as usize] as i32 as i64;
            let product = a.wrapping_mul(b);
            let result = product as u64;
            state.gpr[rt as usize] = result;
            if oe {
                state.set_xer_ov(product < i32::MIN as i64 || product > i32::MAX as i64);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:57 s:3.3.8] mulhwu: high 32 bits of unsigned 32x32 product, zero-extended.
        PpuInstruction::Mulhwu { rt, ra, rb, rc } => {
            let a = state.gpr[ra as usize] as u32 as u64;
            let b = state.gpr[rb as usize] as u32 as u64;
            let result = (a * b) >> 32;
            state.gpr[rt as usize] = result;
            if rc {
                // RT is the unsigned high half (upper 32 bits of RT
                // are zero); CR0 reads the same value, so a high-bit
                // result is positive, not negative.
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:57 s:3.3.8] mulhw: high 32 bits of signed 32x32 product, sign-extended.
        PpuInstruction::Mulhw { rt, ra, rb, rc } => {
            let a = state.gpr[ra as usize] as i32 as i64;
            let b = state.gpr[rb as usize] as i32 as i64;
            let result = ((a * b) >> 32) as i32 as i64 as u64;
            state.gpr[rt as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:57 s:3.3.8] mulhdu: high 64 bits of unsigned 64x64 product.
        PpuInstruction::Mulhdu { rt, ra, rb, rc } => {
            let a = state.gpr[ra as usize] as u128;
            let b = state.gpr[rb as usize] as u128;
            let result = ((a * b) >> 64) as u64;
            state.gpr[rt as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:57 s:3.3.8] mulhd: high 64 bits of signed 64x64 product.
        PpuInstruction::Mulhd { rt, ra, rb, rc } => {
            let a = state.gpr[ra as usize] as i64 as i128;
            let b = state.gpr[rb as usize] as i64 as i128;
            let result = ((a * b) >> 64) as u64;
            state.gpr[rt as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:54 s:3.3.8] adde: (RA) + (RB) + CA; carry-in from XER[CA], CA written.
        PpuInstruction::Adde { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum1, c1) = a.overflowing_add(b);
            let (sum2, c2) = sum1.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum2;
            state.set_xer_ca(c1 || c2);
            if oe {
                let ov = ((a ^ sum2) & (b ^ sum2)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(sum2);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:55 s:3.3.8] addze: (RA) + CA + 0; CA propagated from XER and updated.
        PpuInstruction::Addze { rt, ra, oe, rc } => {
            let a = state.gpr[ra as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum, c) = a.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum;
            state.set_xer_ca(c);
            if oe {
                let ov = ((a ^ sum) & (ca_in ^ sum)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(sum);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:55 s:3.3.8] subfze: ~(RA) + CA + 0; symmetric to addze with RA inverted.
        PpuInstruction::Subfze { rt, ra, oe, rc } => {
            let a = !state.gpr[ra as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum, c) = a.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum;
            state.set_xer_ca(c);
            if oe {
                let ov = ((a ^ sum) & (ca_in ^ sum)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(sum);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:55 s:3.3.8] subfme: ~(RA) + CA + (-1); -1 is u64::MAX in two's complement.
        PpuInstruction::Subfme { rt, ra, oe, rc } => {
            let a = !state.gpr[ra as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum1, c1) = a.overflowing_add(u64::MAX);
            let (sum2, c2) = sum1.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum2;
            state.set_xer_ca(c1 || c2);
            if oe {
                // OV uses the i64 add overflow rule across the
                // two-step `a + (-1) + ca_in`; the equivalent
                // single-step is `a + (ca_in - 1)`.
                let b = u64::MAX;
                let ov = ((a ^ sum2) & (b ^ sum2)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(sum2);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:55 s:3.3.8] addme: (RA) + CA + (-1).
        PpuInstruction::Addme { rt, ra, oe, rc } => {
            let a = state.gpr[ra as usize];
            let ca_in: u64 = state.xer_ca() as u64;
            let (sum1, c1) = a.overflowing_add(u64::MAX);
            let (sum2, c2) = sum1.overflowing_add(ca_in);
            state.gpr[rt as usize] = sum2;
            state.set_xer_ca(c1 || c2);
            if oe {
                let b = u64::MAX;
                let ov = ((a ^ sum2) & (b ^ sum2)) as i64 >> 63 != 0;
                state.set_xer_ov(ov);
            }
            if rc {
                state.set_cr0_from_result(sum2);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:58 s:3.3.8] divw: signed 32-bit divide; RT undefined on overflow (we yield 0).
        PpuInstruction::Divw { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            let overflow = b == 0 || (a == i32::MIN && b == -1);
            let result = if overflow { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as i64 as u64;
            if oe {
                state.set_xer_ov(overflow);
            }
            if rc {
                state.set_cr0_from_result(result as i64 as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:59 s:3.3.8] divwu: unsigned 32-bit divide; sets OV on divide-by-zero.
        PpuInstruction::Divwu { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize] as u32;
            let b = state.gpr[rb as usize] as u32;
            let overflow = b == 0;
            let result = if overflow { 0 } else { a / b };
            state.gpr[rt as usize] = result as u64;
            if oe {
                state.set_xer_ov(overflow);
            }
            if rc {
                // RT is unsigned, zero-extended; CR0 reads the same
                // value (a high-bit result is positive, not negative).
                state.set_cr0_from_result(result as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:58 s:3.3.8] divd: signed 64-bit divide; OV on b=0 or i64::MIN/-1.
        PpuInstruction::Divd { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            let overflow = b == 0 || (a == i64::MIN && b == -1);
            let result = if overflow { 0 } else { a.wrapping_div(b) };
            state.gpr[rt as usize] = result as u64;
            if oe {
                state.set_xer_ov(overflow);
            }
            if rc {
                state.set_cr0_from_result(result as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:59 s:3.3.8] divdu: unsigned 64-bit divide; OV when divisor is zero.
        PpuInstruction::Divdu { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            let overflow = b == 0;
            let result = if overflow { 0 } else { a / b };
            state.gpr[rt as usize] = result;
            if oe {
                state.set_xer_ov(overflow);
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:56 s:3.3.8] mulld: signed 64x64 product, low 64 bits placed into RT.
        PpuInstruction::Mulld { rt, ra, rb, oe, rc } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            let result = a.wrapping_mul(b) as u64;
            state.gpr[rt as usize] = result;
            if oe {
                state.set_xer_ov(a.checked_mul(b).is_none());
            }
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }

        // Logical
        // [PPC-Book1 p:67 s:3.3.11] or: RA <- (RS) | (RB); X-form bit-parallel OR.
        PpuInstruction::Or { ra, rs, rb, rc } => {
            let result = state.gpr[rs as usize] | state.gpr[rb as usize];
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:68 s:3.3.11] orc: RA <- (RS) | ~(RB); OR with complement.
        PpuInstruction::Orc { ra, rs, rb, rc } => {
            let result = state.gpr[rs as usize] | !state.gpr[rb as usize];
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:67 s:3.3.11] and: RA <- (RS) & (RB); X-form bit-parallel AND.
        PpuInstruction::And { ra, rs, rb, rc } => {
            let result = state.gpr[rs as usize] & state.gpr[rb as usize];
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:68 s:3.3.11] nor: RA <- ~((RS) | (RB)); NOR.
        PpuInstruction::Nor { ra, rs, rb, rc } => {
            let result = !(state.gpr[rs as usize] | state.gpr[rb as usize]);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:68 s:3.3.11] andc: RA <- (RS) & ~(RB); AND with complement.
        PpuInstruction::Andc { ra, rs, rb, rc } => {
            let result = state.gpr[rs as usize] & !state.gpr[rb as usize];
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:67 s:3.3.11] xor: RA <- (RS) XOR (RB); X-form bit-parallel XOR.
        PpuInstruction::Xor { ra, rs, rb, rc } => {
            let result = state.gpr[rs as usize] ^ state.gpr[rb as usize];
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:65 s:3.3.11] eqv: RA <- ~((RS) XOR (RB)); bit-parallel XNOR.
        PpuInstruction::Eqv { ra, rs, rb, rc } => {
            let result = !(state.gpr[rs as usize] ^ state.gpr[rb as usize]);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:65 s:3.3.11] nand: RA <- ~((RS) & (RB)); bit-parallel NAND.
        PpuInstruction::Nand { ra, rs, rb, rc } => {
            let result = !(state.gpr[rs as usize] & state.gpr[rb as usize]);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:65 s:3.3.11] andi.: RA <- (RS) & (zero-ext UI); always updates CR0.
        PpuInstruction::AndiDot { ra, rs, imm } => {
            let result = state.gpr[rs as usize] & imm as u64;
            state.gpr[ra as usize] = result;
            state.set_cr0_from_result(result);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:65 s:3.3.11] andis.: RA <- (RS) & (UI << 16); always updates CR0.
        PpuInstruction::AndisDot { ra, rs, imm } => {
            // andis. masks RS with (UI << 16); UI is zero-extended,
            // so high bits of the result above bit 31 stay clear.
            let result = state.gpr[rs as usize] & ((imm as u64) << 16);
            state.gpr[ra as usize] = result;
            state.set_cr0_from_result(result);
            ExecuteVerdict::Continue
        }

        // Shifts
        // [PPC-Book1 p:77 s:3.3.12.2] slw: shift left word; RB[58] selects 32+ shift -> zero result.
        PpuInstruction::Slw { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val << shift } else { 0 } as u64;
            state.gpr[ra as usize] = result;
            if rc {
                // CR0 from a 32-bit word result: sign-extend to 64
                // bits before the LT/GT/EQ comparison. A strict
                // reading of the 64-bit-mode spec would compare the
                // full RA value (always non-negative since
                // RA[0:31]=0), making a result of 0x8000_0000 read
                // as GT. The Cell PPE and RPCS3 both treat the
                // word-width Rc results as sign-extended -- the SR
                // mode-dependency annotation in the opcode table
                // covers this -- and our cross-runner tests rely on
                // it. Do not "fix" to the unsigned reading; that
                // breaks RPCS3 baseline agreement.
                //
                // Same choice applies to Srw / Rlwinm / Rlwimi /
                // Rlwnm below.
                // [PPC-Book1 p:71 s:3.3.12] Rotate/Shift Rc=1: first three CR0 bits set per 3.3.7 result test.
                // [PPC-Book1 p:50 s:3.3.7] CR0 LT/GT/EQ read the (sign-extended) result as a signed value.
                state.set_cr0_from_result(result as i32 as i64 as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:78 s:3.3.12.2] srw: shift right word logical; shift count from RB[58:63].
        PpuInstruction::Srw { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as u32;
            let result = if shift < 32 { val >> shift } else { 0 } as u64;
            state.gpr[ra as usize] = result;
            if rc {
                // Word-width Rc sign-extension; see Slw arm.
                state.set_cr0_from_result(result as i32 as i64 as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:79 s:3.3.12.2] srawi: arithmetic right shift word immediate; CA from shifted-out 1s.
        PpuInstruction::Srawi { ra, rs, sh, rc } => {
            let val = state.gpr[rs as usize] as i32;
            let result = val >> sh;
            let ca = val < 0 && sh > 0 && (val as u32) << (32 - sh) != 0;
            let result_u = result as i64 as u64;
            state.gpr[ra as usize] = result_u;
            state.set_xer_ca(ca);
            if rc {
                state.set_cr0_from_result(result_u);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:79 s:3.3.12.2] sraw: arithmetic right shift word; sign replicated, CA from lost 1s.
        PpuInstruction::Sraw { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x3F;
            let val = state.gpr[rs as usize] as i32;
            let (result, ca) = if shift < 32 {
                let r = val >> shift;
                let ca = val < 0 && shift > 0 && (val as u32) << (32 - shift as u32) != 0;
                (r, ca)
            } else {
                (val >> 31, val < 0)
            };
            let result_u = result as i64 as u64;
            state.gpr[ra as usize] = result_u;
            state.set_xer_ca(ca);
            if rc {
                state.set_cr0_from_result(result_u);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:80 s:3.3.12.2] srad: arithmetic right shift doubleword; CA from lost 1-bits.
        PpuInstruction::Srad { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let val = state.gpr[rs as usize] as i64;
            let (result, ca) = if shift < 64 {
                let r = val >> shift;
                let ca = val < 0 && shift > 0 && (val as u64) << (64 - shift) != 0;
                (r, ca)
            } else {
                (val >> 63, val < 0)
            };
            state.gpr[ra as usize] = result as u64;
            state.set_xer_ca(ca);
            if rc {
                state.set_cr0_from_result(result as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:79 s:3.3.12.2] sradi: arithmetic right shift doubleword immediate.
        PpuInstruction::Sradi { ra, rs, sh, rc } => {
            let shift = sh as u64;
            let val = state.gpr[rs as usize] as i64;
            let result = val >> shift;
            let ca = val < 0 && shift > 0 && (val as u64) << (64 - shift) != 0;
            state.gpr[ra as usize] = result as u64;
            state.set_xer_ca(ca);
            if rc {
                state.set_cr0_from_result(result as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:77 s:3.3.12.2] sld: shift left doubleword; RB[57] selects 64+ -> zero.
        PpuInstruction::Sld { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] << shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:78 s:3.3.12.2] srd: shift right doubleword logical; RB[57] selects 64+ -> zero.
        PpuInstruction::Srd { ra, rs, rb, rc } => {
            let shift = state.gpr[rb as usize] & 0x7F;
            let result = if shift < 64 {
                state.gpr[rs as usize] >> shift
            } else {
                0
            };
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:70 s:3.3.11] cntlzw: count leading zeros of low 32 bits of RS, range 0..=32.
        PpuInstruction::Cntlzw { ra, rs, rc } => {
            let val = state.gpr[rs as usize] as u32;
            let result = val.leading_zeros() as u64;
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:70 s:3.3.11] cntlzd: count leading zeros of 64-bit RS, range 0..=64.
        PpuInstruction::Cntlzd { ra, rs, rc } => {
            let result = state.gpr[rs as usize].leading_zeros() as u64;
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:70 s:3.3.13] popcntb is defined by Book I, but
        // [CBE-Handbook p:738 s:A.2.4.1] lists it as one of the Book-I
        // instructions the Cell PPE does NOT implement; real hardware
        // traps. The oracle mirrors the PPE by faulting rather than
        // computing a per-byte popcount.
        PpuInstruction::Popcntb { ra: _, rs: _ } => {
            ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(122))
        }
        // [PPC-Book1 p:64 s:3.3.10] tw: signed/unsigned 32-bit compare RA[32:63] vs RB[32:63], trap if any TO-selected condition holds.
        PpuInstruction::Tw { to, ra, rb } => {
            let a = state.gpr[ra as usize] as u32 as i32;
            let b = state.gpr[rb as usize] as u32 as i32;
            if trap_condition_matches(to, a as i64, b as i64, a as u32 as u64, b as u32 as u64) {
                ExecuteVerdict::Fault(PpuFault::ProgramTrap(to))
            } else {
                ExecuteVerdict::Continue
            }
        }
        // [PPC-Book1 p:64 s:3.3.10] td: 64-bit signed/unsigned compare RA vs RB, trap if any TO-selected condition holds.
        PpuInstruction::Td { to, ra, rb } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            if trap_condition_matches(to, a, b, a as u64, b as u64) {
                ExecuteVerdict::Fault(PpuFault::ProgramTrap(to))
            } else {
                ExecuteVerdict::Continue
            }
        }
        // [PPC-Book1 p:135 s:6.1] mcrxr: copy XER[32:35] (SO, OV, CA, reserved) into CR field BF, then clear XER[32:35].
        PpuInstruction::Mcrxr { bf } => {
            let nib = ((state.xer >> 28) & 0xF) as u8;
            state.set_cr_field(bf, nib);
            state.xer &= !(0xFu64 << 28);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:69 s:3.3.11] extsh: sign-extend halfword RS[48:63] into RA.
        PpuInstruction::Extsh { ra, rs, rc } => {
            let result = state.gpr[rs as usize] as i16 as i64 as u64;
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:69 s:3.3.11] extsb: sign-extend byte RS[56:63] into RA.
        PpuInstruction::Extsb { ra, rs, rc } => {
            let result = state.gpr[rs as usize] as i8 as i64 as u64;
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:69 s:3.3.11] extsw: sign-extend word RS[32:63] into RA.
        PpuInstruction::Extsw { ra, rs, rc } => {
            let result = state.gpr[rs as usize] as i32 as i64 as u64;
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:66 s:3.3.11] ori: RA <- (RS) | zero-ext UI; ori 0,0,0 is the preferred no-op.
        PpuInstruction::Ori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | imm as u64;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:66 s:3.3.11] oris: RA <- (RS) | (UI << 16).
        PpuInstruction::Oris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] | ((imm as u64) << 16);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:66 s:3.3.11] xori: RA <- (RS) XOR zero-ext UI.
        PpuInstruction::Xori { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ imm as u64;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:66 s:3.3.11] xoris: RA <- (RS) XOR (UI << 16).
        PpuInstruction::Xoris { ra, rs, imm } => {
            state.gpr[ra as usize] = state.gpr[rs as usize] ^ ((imm as u64) << 16);
            ExecuteVerdict::Continue
        }

        // Compare CR-field write: CR[4*BF .. 4*BF+3] <- c || XER_SO,
        // i.e. the SO bit is concatenated as the LSB of every compare
        // result. Without it, code that branches on SO after a
        // compare sees stale data.
        // [PPC-Book1 p:60 s:3.3.9] cmpi/cmpwi (L=0): signed compare 32-bit RA vs SI into CR field BF.
        PpuInstruction::Cmpwi { bf, ra, imm } => {
            let a = state.gpr[ra as usize] as i32;
            let b = imm as i32;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:61 s:3.3.9] cmpli/cmplwi (L=0): unsigned compare 32-bit RA vs zero-ext UI.
        PpuInstruction::Cmplwi { bf, ra, imm } => {
            let a = state.gpr[ra as usize] as u32;
            let b = imm as u32;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:60 s:3.3.9] cmpi/cmpdi (L=1): signed compare 64-bit RA vs SI.
        PpuInstruction::Cmpdi { bf, ra, imm } => {
            let a = state.gpr[ra as usize] as i64;
            let b = imm as i64;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:61 s:3.3.9] cmpli/cmpldi (L=1): unsigned compare 64-bit RA vs zero-ext UI.
        PpuInstruction::Cmpldi { bf, ra, imm } => {
            let a = state.gpr[ra as usize];
            let b = imm as u64;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:60 s:3.3.9] cmp/cmpw (L=0): signed compare of 32-bit RA vs RB.
        PpuInstruction::Cmpw { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as i32;
            let b = state.gpr[rb as usize] as i32;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:61 s:3.3.9] cmpl/cmplw (L=0): unsigned compare of 32-bit RA vs RB.
        PpuInstruction::Cmplw { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as u32;
            let b = state.gpr[rb as usize] as u32;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:60 s:3.3.9] cmp/cmpd (L=1): signed compare of 64-bit RA vs RB.
        PpuInstruction::Cmpd { bf, ra, rb } => {
            let a = state.gpr[ra as usize] as i64;
            let b = state.gpr[rb as usize] as i64;
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:61 s:3.3.9] cmpl/cmpld (L=1): unsigned compare of 64-bit RA vs RB.
        PpuInstruction::Cmpld { bf, ra, rb } => {
            let a = state.gpr[ra as usize];
            let b = state.gpr[rb as usize];
            state.set_cr_field(bf, cmp_cr_field(a < b, a > b, state.xer_so()));
            ExecuteVerdict::Continue
        }

        // CR / SPR moves
        // [PPC-Book2 p:30 s:4.1] mftb: read 64-bit Time Base register into RT (TBR=268).
        PpuInstruction::Mftb { rt } => {
            // Coarse-granularity advance: the per-step resync in
            // `PpuExecutionUnit::run_until_yield` aligns TB with the
            // global tick counter; the +1 here keeps two adjacent
            // mftb reads within the same step strictly increasing,
            // so a guest using a `delta = t2 - t1` idiom never
            // observes zero.
            state.tb = state.tb.saturating_add(1);
            state.gpr[rt as usize] = state.tb;
            ExecuteVerdict::Continue
        }
        // [PPC-Book2 p:30 s:4.1] mftbu: read TBU (high 32 bits of Time Base) into RT[32:63] (TBR=269).
        PpuInstruction::Mftbu { rt } => {
            state.tb = state.tb.saturating_add(1);
            state.gpr[rt as usize] = (state.tb >> 32) & 0xFFFF_FFFF;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:83 s:3.3.13] mfcr: RT[32:63] <- CR; high 32 bits of RT cleared.
        PpuInstruction::Mfcr { rt } => {
            state.gpr[rt as usize] = state.cr as u64;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:83 s:3.3.13] mtcrf: write CR fields selected by FXM mask from RS[32:63].
        PpuInstruction::Mtcrf { rs, crm } => {
            // Bits 32:63 of RS (the low 32 bits in little-endian
            // Rust terms) are placed into selected CR fields. Each
            // bit in CRM selects a 4-bit CR field.
            let val = state.gpr[rs as usize] as u32;
            for i in 0..8u8 {
                if crm & (1 << (7 - i)) != 0 {
                    let shift = (7 - i) * 4;
                    let field_bits = (val >> shift) & 0xF;
                    let mask = 0xF << shift;
                    state.cr = (state.cr & !mask) | (field_bits << shift);
                }
            }
            ExecuteVerdict::Continue
        }
        // [CBE-Handbook p:738 s:A.2.3.1] mfocrf reads ONE CR field.
        // The CRM (FXM) must be one-hot; the set bit selects the
        // field. RT gets that 4-bit field placed at its canonical
        // PowerPC position (CR field N -> RT bits 32+4N..35+4N in
        // PPC numbering = LSB-0 bits (7-N)*4..(7-N)*4+3) and zeros
        // elsewhere. Non-one-hot CRM is boundedly undefined per
        // PPC ISA; RPCS3 throws on that case, so we fault via
        // UnimplementedInstruction(19) to keep the differential
        // harness divergence loud rather than silently masquerading
        // as mfcr.
        PpuInstruction::Mfocrf { rt, crm } => {
            if crm == 0 || crm.count_ones() != 1 {
                return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(19));
            }
            // One-hot CRM: the single set bit's position (MSB-first,
            // 0..=7) selects the CR field index.
            let n = crm.leading_zeros() as u8;
            let field = state.cr_field(n) as u32;
            let shift = (7 - n) * 4;
            state.gpr[rt as usize] = (field << shift) as u64;
            ExecuteVerdict::Continue
        }
        // [CBE-Handbook p:738 s:A.2.3.1] mtocrf writes ONE CR field.
        // CRM is supposed to be one-hot; for non-one-hot CRM the
        // spec says CR is boundedly undefined. RPCS3 deterministic-
        // ally picks the highest set bit
        // (`countl_zero(crm) & 7`); we mirror that so the
        // differential harness matches and the executor is NOT a
        // passthrough to mtcrf (which would update every selected
        // field, not just one).
        PpuInstruction::Mtocrf { rs, crm } => {
            if crm == 0 {
                // No CRM bit selected: nothing to write. Distinct
                // from mtcrf with crm == 0 (also nop), but reached
                // via a different decode path.
                return ExecuteVerdict::Continue;
            }
            let n = crm.leading_zeros() as u8; // highest set bit, 0..=7
            let val = state.gpr[rs as usize] as u32;
            let shift = (7 - n) * 4;
            let field_bits = ((val >> shift) & 0xF) as u8;
            state.set_cr_field(n, field_bits);
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:82 s:3.3.13] mflr: extended mnemonic for mfspr RT,LR (SPR 8).
        PpuInstruction::Mflr { rt } => {
            state.gpr[rt as usize] = state.lr;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:82 s:3.3.13] mtlr: extended mnemonic for mtspr LR,RS (SPR 8).
        PpuInstruction::Mtlr { rs } => {
            state.lr = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:82 s:3.3.13] mfctr: extended mnemonic for mfspr RT,CTR (SPR 9).
        PpuInstruction::Mfctr { rt } => {
            state.gpr[rt as usize] = state.ctr;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:82 s:3.3.13] mtctr: extended mnemonic for mtspr CTR,RS (SPR 9).
        PpuInstruction::Mtctr { rs } => {
            state.ctr = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:82 s:3.3.13] mfxer: extended mnemonic for mfspr RT,XER (SPR 1).
        PpuInstruction::Mfxer { rt } => {
            state.gpr[rt as usize] = state.xer;
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:81 s:3.3.13] mtxer: extended mnemonic for mtspr XER,RS (SPR 1); writes the architecturally-defined fields of XER.
        PpuInstruction::Mtxer { rs } => {
            state.xer = state.gpr[rs as usize];
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:48 s:2.3.2 VRSAVE Register] mfvrsave:
        // extended mnemonic for mfspr RT, VRSAVE (SPR 256). Reads
        // the 32-bit VR-usage mask into rT; the upper 32 bits of
        // rT are zero (standard widening for a 32-bit SPR read).
        //
        // Tripwire: this read guards the sync_state_hash exclusion
        // of VRSAVE. The exclusion is sound only while VRSAVE is
        // write-only-then-read-back -- a cold read consumes the
        // seed (0) as architectural state, and a divergent seed
        // would not have surfaced through any prior GPR diff. If
        // this assert ever trips, reopen the exclusion decision
        // (see the VRSAVE note in state.rs) before trusting the
        // title's differential results. NOT a fault: reading
        // VRSAVE before writing is legal PPC; faulting would
        // impersonate a non-existent architectural error.
        //
        // The counter increments *before* the assert so the
        // witness records the read even on a run that trips.
        PpuInstruction::Mfvrsave { rt } => {
            state.mfvrsave_executed = state.mfvrsave_executed.wrapping_add(1);
            debug_assert!(
                state.vrsave_written,
                "mfvrsave of never-written VRSAVE: title consumes seed VRSAVE (0) as \
                 architectural state. The sync_state_hash exclusion of VRSAVE assumes it \
                 is write-only-then-read-back (inert save/restore idiom); a cold read \
                 breaks that precondition because a divergent seed would not surface \
                 through any prior GPR diff. Reopen the VRSAVE hash-inclusion decision \
                 (see the VRSAVE exclusion note in state.rs) before trusting this title's \
                 differential results."
            );
            state.gpr[rt as usize] = state.vrsave as u64;
            ExecuteVerdict::Continue
        }
        // [AltiVec-PEM p:48 s:2.3.2 VRSAVE Register] mtvrsave:
        // extended mnemonic for mtspr VRSAVE, RS (SPR 256). Writes
        // the low 32 bits of rS into VRSAVE; the upper 32 bits of
        // rS are architecturally ignored (VRSAVE is 32-bit). Marks
        // vrsave_written so the Mfvrsave tripwire can tell write-
        // before-read from cold read.
        PpuInstruction::Mtvrsave { rs } => {
            state.vrsave = state.gpr[rs as usize] as u32;
            state.vrsave_written = true;
            ExecuteVerdict::Continue
        }

        // Rotate / mask
        // [PPC-Book1 p:73 s:3.3.12] rlwinm: rotate left 32 bits by SH, AND with mask MB..ME.
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask = rlwinm_mask(mb, me);
            let result = (rotated & mask) as u64;
            state.gpr[ra as usize] = result;
            if rc {
                // Word-width Rc sign-extension; see Slw arm.
                state.set_cr0_from_result(result as i32 as i64 as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:76 s:3.3.12] rlwimi: rotate left word, insert under
        // mask MB..ME into RA. Spec's RA <- r&m | (RA)&~m operates on 64-bit
        // operands; mask MASK(MB+32, ME+32) only has 1-bits in the low 32, so
        // the high 32 of RA must be PRESERVED. Earlier code cast prior RA to
        // u32 before merging and zero-extended back, wiping RA[0:31]; cross-
        // checked against RPCS3 PPUInterpreter.cpp's `(gpr[ra] & ~mask) |
        // (dup32(rotl) & mask)` form.
        PpuInstruction::Rlwimi {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => {
            let val = state.gpr[rs as usize] as u32;
            let rotated = val.rotate_left(sh as u32);
            let mask_lo32 = rlwinm_mask(mb, me);
            let mask64 = u64::from(mask_lo32);
            let merged = (u64::from(rotated) & mask64) | (state.gpr[ra as usize] & !mask64);
            state.gpr[ra as usize] = merged;
            if rc {
                state.set_cr0_from_result(merged);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:75 s:3.3.12] rlwnm: rotate left word by RB[59:63], AND with mask MB..ME.
        PpuInstruction::Rlwnm {
            ra,
            rs,
            rb,
            mb,
            me,
            rc,
        } => {
            let val = state.gpr[rs as usize] as u32;
            let n = (state.gpr[rb as usize] & 0x1F) as u32;
            let rotated = val.rotate_left(n);
            let mask = rlwinm_mask(mb, me);
            let result = (rotated & mask) as u64;
            state.gpr[ra as usize] = result;
            if rc {
                // Word-width Rc sign-extension; see Slw arm.
                state.set_cr0_from_result(result as i32 as i64 as u64);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:72 s:3.3.12] rldicl: rotate left doubleword immediate, mask MB..63 (clear left).
        PpuInstruction::Rldicl { ra, rs, sh, mb, rc } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            let result = rotated & mask64(mb, 63);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:72 s:3.3.12] rldicr: rotate left doubleword immediate, mask 0..ME (clear right).
        PpuInstruction::Rldicr { ra, rs, sh, me, rc } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            let result = rotated & mask64(0, me);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:73 s:3.3.12] rldic: rotate left doubleword imm, mask MB..63-SH (clear).
        PpuInstruction::Rldic { ra, rs, sh, mb, rc } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            let me = 63u8.saturating_sub(sh);
            let result = rotated & mask64(mb, me);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:76 s:3.3.12] rldimi: rotate left doubleword imm, insert under mask MB..63-SH.
        PpuInstruction::Rldimi { ra, rs, sh, mb, rc } => {
            let rotated = state.gpr[rs as usize].rotate_left(sh as u32);
            let me = 63u8.saturating_sub(sh);
            let mask = mask64(mb, me);
            let prior = state.gpr[ra as usize];
            let result = (rotated & mask) | (prior & !mask);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:75 s:3.3.12] rldcl: ROTL64(RS, RB[58:63]) & MASK(mb, 63).
        PpuInstruction::Rldcl { ra, rs, rb, mb, rc } => {
            let sh = (state.gpr[rb as usize] & 0x3F) as u32;
            let rotated = state.gpr[rs as usize].rotate_left(sh);
            let result = rotated & mask64(mb, 63);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }
        // [PPC-Book1 p:75 s:3.3.12] rldcr: ROTL64(RS, RB[58:63]) & MASK(0, me).
        PpuInstruction::Rldcr { ra, rs, rb, me, rc } => {
            let sh = (state.gpr[rb as usize] & 0x3F) as u32;
            let rotated = state.gpr[rs as usize].rotate_left(sh);
            let result = rotated & mask64(0, me);
            state.gpr[ra as usize] = result;
            if rc {
                state.set_cr0_from_result(result);
            }
            ExecuteVerdict::Continue
        }

        _ => unreachable!("alu::execute called with non-ALU variant"),
    }
}

/// AND each TO bit against its corresponding `tw`/`td` comparison
/// outcome and return whether any condition fires.
///
/// TO bits per PPC numbering (MSB-first within the 5-bit field):
/// bit 0 = signed less, bit 1 = signed greater, bit 2 = equal,
/// bit 3 = unsigned less, bit 4 = unsigned greater. Our `to: u8`
/// holds the field with bit 4 (the MSB) = signed-less.
// [PPC-Book1 p:62 s:3.3.10] TO-bit-to-condition mapping for tw / td.
fn trap_condition_matches(
    to: u8,
    a_signed: i64,
    b_signed: i64,
    a_unsigned: u64,
    b_unsigned: u64,
) -> bool {
    ((to >> 4) & 1 != 0 && a_signed < b_signed)
        || ((to >> 3) & 1 != 0 && a_signed > b_signed)
        || ((to >> 2) & 1 != 0 && a_signed == b_signed)
        || ((to >> 1) & 1 != 0 && a_unsigned < b_unsigned)
        || (to & 1 != 0 && a_unsigned > b_unsigned)
}

/// Build a 4-bit CR field for compare instructions: `LT|GT|EQ|SO`.
/// Exactly one of `lt`/`gt`/`eq` is set; `so` is the sticky overflow
/// bit copied unchanged from XER.
// [PPC-Book1 p:60 s:3.3.9] CR field encoding c||XER[SO] -> {LT,GT,EQ,SO} nibble.
fn cmp_cr_field(lt: bool, gt: bool, so: bool) -> u8 {
    let mut nib = if lt {
        0b1000
    } else if gt {
        0b0100
    } else {
        0b0010
    };
    if so {
        nib |= 0b0001;
    }
    nib
}

/// 32-bit rlwinm mask. `mb > me` wraps to bits `[0..me]` and `[mb..31]`.
// [PPC-Book1 p:11 s:1.7] M-form MB/ME mask: 1s from MB to ME inclusive, wraps when MB>ME.
pub(super) fn rlwinm_mask(mb: u8, me: u8) -> u32 {
    if mb <= me {
        let top = 0xFFFF_FFFFu32 >> mb;
        let bottom = 0xFFFF_FFFFu32 << (31 - me);
        top & bottom
    } else {
        let top = 0xFFFF_FFFFu32 << (31 - me);
        let bottom = 0xFFFF_FFFFu32 >> mb;
        top | bottom
    }
}

/// 64-bit PPC mask from MSB-numbered bits `mb..=me`; `mb > me` wraps
/// to `[0..me]` and `[mb..63]`.
// [PPC-Book1 p:71 s:3.3.12] MD/MDS-form 64-bit MASK function: bits[mb:me] = 1, wrapping.
fn mask64(mb: u8, me: u8) -> u64 {
    let all = 0xFFFF_FFFF_FFFF_FFFFu64;
    if mb <= me {
        let top = all >> mb;
        let bottom = all << (63 - me);
        top & bottom
    } else {
        let top = all << (63 - me);
        let bottom = all >> mb;
        top | bottom
    }
}

#[cfg(test)]
#[path = "tests/alu_tests.rs"]
mod tests;
