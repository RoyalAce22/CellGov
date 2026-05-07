//! Arithmetic, logical, shift, rotate, compare, CR/SPR-move dispatch.
//! Every arm here is a pure register-to-register or register-to-CR
//! operation; nothing in this module touches memory or emits effects.

use crate::exec::ExecuteVerdict;
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
        // [PPC-Book1 p:76 s:3.3.12] rlwimi: rotate left word, insert under mask MB..ME into RA.
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
            let mask = rlwinm_mask(mb, me);
            let prior = state.gpr[ra as usize] as u32;
            let merged = ((rotated & mask) | (prior & !mask)) as u64;
            state.gpr[ra as usize] = merged;
            if rc {
                // Word-width Rc sign-extension; see Slw arm.
                state.set_cr0_from_result(merged as i32 as i64 as u64);
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

        _ => unreachable!("alu::execute called with non-ALU variant"),
    }
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
mod tests {
    use super::*;
    use crate::exec::test_support::exec_no_mem;

    #[test]
    fn rlwinm_mask_contiguous() {
        assert_eq!(rlwinm_mask(0, 31), 0xFFFFFFFF);
        assert_eq!(rlwinm_mask(16, 31), 0x0000FFFF);
        assert_eq!(rlwinm_mask(0, 15), 0xFFFF0000);
    }

    #[test]
    fn rlwinm_mask_wrapped() {
        // mb > me: mask wraps around; here bits [0..3] and [28..31].
        assert_eq!(rlwinm_mask(28, 3), 0xF000000F);
    }

    #[test]
    fn addi_with_ra_zero_is_li() {
        let mut s = PpuState::new();
        exec_no_mem(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 0,
                imm: 42,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 42);
    }

    #[test]
    fn addi_with_ra_nonzero_adds() {
        let mut s = PpuState::new();
        s.gpr[5] = 100;
        exec_no_mem(
            &PpuInstruction::Addi {
                rt: 3,
                ra: 5,
                imm: -10,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 90);
    }

    #[test]
    fn addis_shifts_left_16() {
        let mut s = PpuState::new();
        exec_no_mem(
            &PpuInstruction::Addis {
                rt: 3,
                ra: 0,
                imm: 1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x10000);
    }

    #[test]
    fn ori_zero_is_move() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xCAFE;
        exec_no_mem(
            &PpuInstruction::Ori {
                ra: 3,
                rs: 5,
                imm: 0,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xCAFE);
    }

    #[test]
    fn cmpwi_sets_cr_field() {
        let mut s = PpuState::new();
        s.gpr[3] = 10;
        exec_no_mem(
            &PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 10,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn mflr_mtlr_roundtrip() {
        let mut s = PpuState::new();
        s.gpr[5] = 0xABCD;
        exec_no_mem(&PpuInstruction::Mtlr { rs: 5 }, &mut s);
        assert_eq!(s.lr, 0xABCD);
        exec_no_mem(&PpuInstruction::Mflr { rt: 3 }, &mut s);
        assert_eq!(s.gpr[3], 0xABCD);
    }

    #[test]
    fn rlwinm_slwi() {
        let mut s = PpuState::new();
        s.gpr[5] = 0x0001;
        // slwi r3, r5, 16 == rlwinm r3, r5, 16, 0, 15
        exec_no_mem(
            &PpuInstruction::Rlwinm {
                ra: 3,
                rs: 5,
                sh: 16,
                mb: 0,
                me: 15,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x10000);
    }

    #[test]
    fn rlwnm_rotates_by_rb_low_5_bits() {
        let mut s = PpuState::new();
        s.gpr[0] = 0x0000_0000_1234_5678;
        s.gpr[8] = 8;
        exec_no_mem(
            &PpuInstruction::Rlwnm {
                ra: 0,
                rs: 0,
                rb: 8,
                mb: 0,
                me: 31,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[0], 0x3456_7812);
    }

    #[test]
    fn rlwnm_ignores_high_bits_of_rb() {
        // 0x20 == 32: only low 5 bits feed the rotate, so rotation == 0.
        let mut s = PpuState::new();
        s.gpr[1] = 0x0000_0000_DEAD_BEEF;
        s.gpr[2] = 0x20;
        exec_no_mem(
            &PpuInstruction::Rlwnm {
                ra: 3,
                rs: 1,
                rb: 2,
                mb: 0,
                me: 31,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    }

    #[test]
    fn extsw_sign_extends_low_32_bits() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x0000_0000_8000_0000;
        exec_no_mem(
            &PpuInstruction::Extsw {
                ra: 4,
                rs: 3,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
    }

    #[test]
    fn divdu_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 7;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 14);
    }

    #[test]
    fn divdu_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn divdu_large_values() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Divdu {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0x7FFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn divd_signed() {
        let mut s = PpuState::new();
        s.gpr[3] = (-100i64) as u64;
        s.gpr[4] = 7;
        exec_no_mem(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5] as i64, -14);
    }

    #[test]
    fn divd_small_dividend_returns_zero() {
        // Hex-format conversion routines do `value / base` until
        // value reaches 0; the last iteration always has dividend
        // < divisor (e.g. 0xF / 16, 1 / 16). Verify those produce
        // zero quotient.
        for (a, b) in [(0u64, 16u64), (1, 16), (0xFu64, 16), (15, 16)] {
            let mut s = PpuState::new();
            s.gpr[3] = a;
            s.gpr[4] = b;
            exec_no_mem(
                &PpuInstruction::Divd {
                    rt: 5,
                    ra: 3,
                    rb: 4,
                    oe: false,
                    rc: false,
                },
                &mut s,
            );
            assert_eq!(s.gpr[5], 0, "divd({a:#x}, {b}) expected 0");
        }
    }

    #[test]
    fn divd_divide_by_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        exec_no_mem(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn mulld_basic() {
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        exec_no_mem(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 56);
    }

    #[test]
    fn mulld_wraps_on_overflow() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Mulld {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        // -1 * 2 = -2 (wrapping) = 0xFFFF_FFFF_FFFF_FFFE
        assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFE);
    }

    #[test]
    fn adde_adds_with_carry_in_and_sets_carry_out() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 0;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert!(s.xer_ca());
    }

    #[test]
    fn adde_without_carry_clears_ca() {
        let mut s = PpuState::new();
        s.gpr[3] = 5;
        s.gpr[4] = 3;
        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::Adde {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 8);
        assert!(!s.xer_ca());
    }

    #[test]
    fn mulhdu_takes_high_64_bits_of_u128_product() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 1);
    }

    #[test]
    fn mulhdu_small_product_is_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 7;
        s.gpr[4] = 8;
        exec_no_mem(
            &PpuInstruction::Mulhdu {
                rt: 5,
                ra: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
    }

    #[test]
    fn mulhw_signed_high_32_bits() {
        let mut s = PpuState::new();
        s.gpr[4] = (-2i32) as u32 as u64;
        s.gpr[5] = 3;
        exec_no_mem(
            &PpuInstruction::Mulhw {
                rt: 3,
                ra: 4,
                rb: 5,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0xFFFFFFFF_FFFFFFFFu64);
    }

    #[test]
    fn mulhw_positive_produces_zero_high_bits() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x0001_0000;
        s.gpr[5] = 0x0001_0000;
        exec_no_mem(
            &PpuInstruction::Mulhw {
                rt: 3,
                ra: 4,
                rb: 5,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 1);
    }

    #[test]
    fn cntlzd_counts_64_for_zero() {
        let mut s = PpuState::new();
        s.gpr[5] = 0;
        exec_no_mem(
            &PpuInstruction::Cntlzd {
                ra: 3,
                rs: 5,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 64);
    }

    #[test]
    fn cntlzd_high_bit_set_returns_zero() {
        let mut s = PpuState::new();
        s.gpr[5] = 1u64 << 63;
        exec_no_mem(
            &PpuInstruction::Cntlzd {
                ra: 3,
                rs: 5,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0);
    }

    #[test]
    fn addze_with_ca_zero_copies_ra() {
        let mut s = PpuState::new();
        s.gpr[4] = 42;
        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::Addze {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 42);
        assert!(!s.xer_ca());
    }

    #[test]
    fn addze_with_ca_set_adds_one() {
        let mut s = PpuState::new();
        s.gpr[4] = 42;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Addze {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 43);
        assert!(!s.xer_ca());
    }

    #[test]
    fn addze_overflow_sets_ca() {
        let mut s = PpuState::new();
        s.gpr[4] = u64::MAX;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Addze {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0);
        assert!(s.xer_ca());
    }

    #[test]
    fn addze_oe_signed_overflow_sets_ov_and_so() {
        // Max positive i64 + CA=1 wraps to min i64: signed overflow.
        let mut s = PpuState::new();
        s.gpr[4] = 0x7FFF_FFFF_FFFF_FFFF;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Addze {
                rt: 3,
                ra: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0x8000_0000_0000_0000);
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
    }

    #[test]
    fn addze_oe_no_overflow_clears_ov_keeps_so_sticky() {
        // u64::MAX + 1 carries (CA=1) and wraps to 0, but signed:
        // -1 + 1 = 0, no signed overflow. OV must be cleared while
        // any pre-existing sticky SO is preserved.
        let mut s = PpuState::new();
        s.gpr[4] = u64::MAX;
        s.set_xer_ca(true);
        // Pre-set sticky SO via set_xer_ov round-trip so the entry
        // state has SO=1, OV=0; the round-trip itself is covered in
        // state.rs tests.
        s.set_xer_ov(true);
        s.set_xer_ov(false);
        exec_no_mem(
            &PpuInstruction::Addze {
                rt: 3,
                ra: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], 0);
        assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO sticky");
    }

    #[test]
    fn orc_is_or_with_complement_rb() {
        let mut s = PpuState::new();
        s.gpr[4] = 0x00FF_0000;
        s.gpr[5] = 0x0000_00FF;
        exec_no_mem(
            &PpuInstruction::Orc {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: false,
            },
            &mut s,
        );
        // orc is 32-bit, result sign-extended to 64 bits on this operand.
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FF00);
    }

    #[test]
    fn subfc_computes_rb_minus_ra_and_sets_ca_on_no_borrow() {
        let mut s = PpuState::new();
        s.gpr[3] = 3;
        s.gpr[4] = 10;
        exec_no_mem(
            &PpuInstruction::Subfc {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 7);
        assert!(s.xer_ca());
    }

    #[test]
    fn subfc_borrow_clears_ca() {
        let mut s = PpuState::new();
        s.gpr[3] = 10;
        s.gpr[4] = 3;
        exec_no_mem(
            &PpuInstruction::Subfc {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 3u64.wrapping_sub(10));
        assert!(!s.xer_ca());
    }

    #[test]
    fn subfe_uses_carry_in() {
        // rt = ~ra + rb + CA: CA=1 gives rb - ra, CA=0 gives rb - ra - 1.
        let mut s = PpuState::new();
        s.gpr[3] = 3;
        s.gpr[4] = 10;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Subfe {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 7);

        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::Subfe {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 6);
    }

    #[test]
    fn sraw_preserves_sign_and_caps_at_31() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_8000_0000;
        s.gpr[4] = 4;
        exec_no_mem(
            &PpuInstruction::Sraw {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5] as i32 as i64, -2147483648i64 >> 4);
    }

    #[test]
    fn srad_signed_64_bit_shift() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x8000_0000_0000_0000;
        s.gpr[4] = 4;
        exec_no_mem(
            &PpuInstruction::Srad {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5] as i64, (0x8000_0000_0000_0000u64 as i64) >> 4);
    }

    #[test]
    fn sradi_shift_zero_clears_ca_and_preserves_value() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xDEAD_BEEF_CAFE_F00D;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Sradi {
                ra: 4,
                rs: 3,
                sh: 0,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_F00D);
        assert!(!s.xer_ca());
    }

    #[test]
    fn mulhd_signed_high_doubleword() {
        let mut s = PpuState::new();
        s.gpr[3] = u64::MAX;
        s.gpr[4] = u64::MAX;
        exec_no_mem(
            &PpuInstruction::Mulhd {
                rt: 5,
                ra: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);

        s.gpr[3] = u64::MAX;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Mulhd {
                rt: 5,
                ra: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], u64::MAX);
    }

    #[test]
    fn cmpdi_compares_full_64_bits() {
        // With only the low 32 bits examined, 0x1_0000_0000 would compare
        // equal to zero. cmpdi must see the full doubleword.
        let mut s = PpuState::new();
        s.gpr[3] = 0x1_0000_0000;
        exec_no_mem(
            &PpuInstruction::Cmpdi {
                bf: 0,
                ra: 3,
                imm: 0,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b0100); // GT
    }

    #[test]
    fn cmpldi_compares_full_64_bits_unsigned() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1_0000_0000;
        exec_no_mem(
            &PpuInstruction::Cmpldi {
                bf: 1,
                ra: 3,
                imm: 0,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(1), 0b0100); // GT
    }

    #[test]
    fn rldic_clears_both_sides() {
        // rldic RA, RS, SH=4, MB=32: rotate left 4, keep bits 32..=(63-4)=59.
        // RS=0xFFFF_FFFF_FFFF_FFFF, rotated left 4 still saturated, mask zeroes
        // bits 0..=31 and 60..=63.
        let mut s = PpuState::new();
        s.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
        exec_no_mem(
            &PpuInstruction::Rldic {
                ra: 5,
                rs: 4,
                sh: 4,
                mb: 32,
                rc: false,
            },
            &mut s,
        );
        // bits 32..=59 set, others clear.
        let expected: u64 = ((1u64 << 28) - 1) << 4;
        assert_eq!(s.gpr[5], expected);
    }

    #[test]
    fn rldimi_preserves_prior_ra_outside_mask() {
        // rldimi RA, RS, SH=16, MB=0: mask = 0..=(63-16)=47, preserve 48..=63.
        let mut s = PpuState::new();
        s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE; // RS
        s.gpr[5] = 0x1111_2222_3333_4444; // prior RA
        exec_no_mem(
            &PpuInstruction::Rldimi {
                ra: 5,
                rs: 4,
                sh: 16,
                mb: 0,
                rc: false,
            },
            &mut s,
        );
        // rotated = RS rotl 16 = 0xBEEF_CAFE_BABE_DEAD
        // mask = 0xFFFF_FFFF_FFFF_0000 (bits 0..=47 set)
        // merged = (rotated & mask) | (prior & !mask)
        //        = 0xBEEF_CAFE_BABE_0000 | 0x0000_0000_0000_4444
        //        = 0xBEEF_CAFE_BABE_4444
        assert_eq!(s.gpr[5], 0xBEEF_CAFE_BABE_4444);
    }

    #[test]
    fn srad_shifts_full_64_bits_arithmetically() {
        let mut s = PpuState::new();
        s.gpr[4] = 0xFFFF_FFFF_FFFF_FFF0; // -16
        s.gpr[5] = 4;
        exec_no_mem(
            &PpuInstruction::Srad {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: false,
            },
            &mut s,
        );
        // -16 >> 4 = -1, sign-extended across all 64 bits.
        assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn mftbu_returns_upper_32_bits_of_tb() {
        let mut s = PpuState::new();
        s.tb = 0xAAAA_BBBB_0000_0000 - 1; // post-increment lands at 0xAAAA_BBBB_0000_0000
        exec_no_mem(&PpuInstruction::Mftbu { rt: 6 }, &mut s);
        assert_eq!(s.gpr[6], 0xAAAA_BBBB);
    }

    #[test]
    fn mftb_returns_strictly_increasing_values_within_step() {
        // Two consecutive mftb reads in the same step must differ so a
        // guest doing `delta = t2 - t1` never observes zero.
        let mut s = PpuState::new();
        s.tb = 100;
        exec_no_mem(&PpuInstruction::Mftb { rt: 3 }, &mut s);
        let t1 = s.gpr[3];
        exec_no_mem(&PpuInstruction::Mftb { rt: 4 }, &mut s);
        let t2 = s.gpr[4];
        assert!(
            t2 > t1,
            "mftb must strictly increase per read: {t1} -> {t2}"
        );
    }

    // -- Rc / OE regression tests --
    // Record form (Rc=1) must set CR0 LT/GT/EQ from the signed 64-bit
    // result, plus the sticky SO from XER. OE=1 must set XER OV and the
    // sticky SO on overflow.

    #[test]
    fn add_dot_sets_cr0_eq_when_result_is_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 1;
        s.gpr[4] = (-1i64) as u64;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn add_dot_sets_cr0_lt_when_result_is_negative() {
        let mut s = PpuState::new();
        s.gpr[3] = 1;
        s.gpr[4] = (-2i64) as u64;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b1000);
    }

    #[test]
    fn add_rc_zero_leaves_cr0_untouched() {
        let mut s = PpuState::new();
        s.set_cr_field(0, 0b0100);
        s.gpr[3] = 1;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b0100, "CR0 preserved when Rc=0");
    }

    #[test]
    fn addo_sets_xer_ov_and_sticky_so() {
        let mut s = PpuState::new();
        s.gpr[3] = i64::MAX as u64;
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");

        // Non-overflow op clears OV but SO stays sticky.
        s.gpr[3] = 1;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO remains sticky");
    }

    #[test]
    fn or_dot_sets_cr0_without_touching_result() {
        // `or. rA, rS, rS` must update CR0; quickening it to plain
        // `Mr` (move register) is incorrect because Mr has no Rc form.
        let mut s = PpuState::new();
        s.gpr[4] = (-5i64) as u64;
        exec_no_mem(
            &PpuInstruction::Or {
                ra: 3,
                rs: 4,
                rb: 4,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[3], (-5i64) as u64);
        assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
    }

    #[test]
    fn and_dot_sets_cr0_eq_on_zero() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFF00;
        s.gpr[4] = 0x00FF;
        exec_no_mem(
            &PpuInstruction::And {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn slw_dot_sets_cr0_from_sign_extended_low_32() {
        // Result is 0x8000_0000 as u32, which sign-extends to a negative
        // i64 -- CR0 should read LT.
        let mut s = PpuState::new();
        s.gpr[3] = 1;
        s.gpr[4] = 31;
        exec_no_mem(
            &PpuInstruction::Slw {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0x8000_0000);
        assert_eq!(s.cr_field(0), 0b1000);
    }

    #[test]
    fn srad_dot_sets_cr0_and_preserves_ca() {
        let mut s = PpuState::new();
        s.gpr[3] = (-1i64) as u64; // all-ones, guaranteed 1-bit shifted out.
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Srad {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: true,
            },
            &mut s,
        );
        // -1 >> 1 = -1, and a 1 bit was shifted out of a negative value: CA set.
        assert!(s.xer_ca(), "CA set from nonzero bits shifted out");
        assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
    }

    #[test]
    fn sradi_dot_sets_cr0() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x8000_0000_0000_0000;
        exec_no_mem(
            &PpuInstruction::Sradi {
                ra: 5,
                rs: 3,
                sh: 8,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b1000);
    }

    #[test]
    fn cntlzd_dot_sets_cr0_gt_when_value_nonzero() {
        let mut s = PpuState::new();
        s.gpr[3] = 1u64 << 40;
        exec_no_mem(
            &PpuInstruction::Cntlzd {
                ra: 5,
                rs: 3,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 23);
        assert_eq!(s.cr_field(0), 0b0100);
    }

    #[test]
    fn rldicl_dot_sets_cr0_and_does_not_quicken_to_clrldi() {
        // Verifies the shadow-layer guard: rldicl. with sh=0 cannot be
        // quickened to Clrldi because Clrldi does not update CR0.
        let mut s = PpuState::new();
        s.gpr[3] = 0;
        exec_no_mem(
            &PpuInstruction::Rldicl {
                ra: 5,
                rs: 3,
                sh: 0,
                mb: 32,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert_eq!(s.cr_field(0), 0b0010);
    }

    #[test]
    fn rldimi_dot_sets_cr0_from_merged_value() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1; // RS
        s.gpr[5] = 0xFFFF_FFFF_FFFF_FFFF; // prior RA (bits outside mask preserved)
                                          // rldimi. rA, rS, 32, 0: mask = bits 0..=31, merge RS<<32 into high half.
        exec_no_mem(
            &PpuInstruction::Rldimi {
                ra: 5,
                rs: 3,
                sh: 32,
                mb: 0,
                rc: true,
            },
            &mut s,
        );
        // rotated = 1 rotl 32 = 0x0000_0001_0000_0000
        // mask = 0xFFFF_FFFF_0000_0000
        // merged = (rotated & mask) | (prior & !mask)
        //        = 0x0000_0001_0000_0000 | 0x0000_0000_FFFF_FFFF
        //        = 0x0000_0001_FFFF_FFFF
        assert_eq!(s.gpr[5], 0x0000_0001_FFFF_FFFF);
        assert_eq!(s.cr_field(0), 0b0100, "positive nonzero");
    }

    #[test]
    fn nego_of_int_min_sets_ov() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x8000_0000_0000_0000;
        exec_no_mem(
            &PpuInstruction::Neg {
                rt: 5,
                ra: 3,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    }

    #[test]
    fn divwo_div_by_zero_sets_ov() {
        let mut s = PpuState::new();
        s.gpr[3] = 100;
        s.gpr[4] = 0;
        exec_no_mem(
            &PpuInstruction::Divw {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
    }

    #[test]
    fn mullwo_with_overflow_sets_ov() {
        let mut s = PpuState::new();
        s.gpr[3] = 0x1_0000;
        s.gpr[4] = 0x1_0000;
        exec_no_mem(
            &PpuInstruction::Mullw {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        // 0x1_0000 * 0x1_0000 = 0x1_0000_0000, overflows 32-bit signed.
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
    }

    #[test]
    fn cr0_so_bit_tracks_sticky_xer_so() {
        // After an overflow, every record-form instruction must copy the
        // current (sticky) SO into CR0.SO.
        let mut s = PpuState::new();
        s.gpr[3] = i64::MAX as u64;
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: false,
            },
            &mut s,
        );
        // SO is set. A subsequent dot-form should carry SO into CR0.
        s.gpr[3] = 1;
        s.gpr[4] = 2;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.cr_field(0), 0b0101, "GT plus sticky SO");
    }

    #[test]
    fn addo_dot_combined_sets_both_ov_and_cr0() {
        // oe=rc=true: executor must act on both bits independently.
        let mut s = PpuState::new();
        s.gpr[3] = i64::MAX as u64;
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Add {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: true,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
        assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
        // Result is INT_MIN, negative -- CR0 = LT plus sticky SO.
        assert_eq!(s.cr_field(0), 0b1001);
    }

    #[test]
    fn srawi_dot_sets_both_ca_and_cr0() {
        let mut s = PpuState::new();
        s.gpr[3] = (-1i32) as u32 as u64;
        exec_no_mem(
            &PpuInstruction::Srawi {
                ra: 5,
                rs: 3,
                sh: 1,
                rc: true,
            },
            &mut s,
        );
        // -1 arithmetic-shift-right-by-1 yields -1; negative RS with a
        // 1-bit shifted out sets CA; Rc sets CR0 LT from the negative result.
        assert!(s.xer_ca());
        assert_eq!(s.cr_field(0), 0b1000);
    }

    #[test]
    fn srawi_sh_zero_clears_ca() {
        // [PPC-Book1 p:80 s:3.3.12.2] "A shift amount of zero causes
        // RA to receive EXTS(RS[32:63]), and CA to be set to 0." CA
        // is explicitly cleared, not computed from the (nonexistent)
        // shifted-out bits.
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Srawi {
                ra: 5,
                rs: 3,
                sh: 0,
                rc: false,
            },
            &mut s,
        );
        assert!(!s.xer_ca(), "sh=0 must clear CA regardless of prior value");
        assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF, "EXTS of -1 low word");
    }

    #[test]
    fn srad_shift_ge_64_collapses_to_sign_broadcast() {
        // shift >= 64: RA = 64 copies of the sign bit, CA = sign bit.
        let mut s = PpuState::new();
        s.gpr[3] = 0x8000_0000_0000_0000;
        s.gpr[4] = 64;
        exec_no_mem(
            &PpuInstruction::Srad {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF);
        assert!(s.xer_ca());

        // shift > 64 with positive RS: all zeros, CA clear.
        s.gpr[3] = 0x1;
        s.gpr[4] = 100;
        exec_no_mem(
            &PpuInstruction::Srad {
                ra: 5,
                rs: 3,
                rb: 4,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0);
        assert!(!s.xer_ca());
    }

    #[test]
    fn subfic_sets_xer_ca_on_no_borrow() {
        let mut s = PpuState::new();
        s.gpr[3] = 5;
        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::Subfic {
                rt: 4,
                ra: 3,
                imm: 10,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 5);
        assert!(s.xer_ca(), "subfic sets CA when there is no borrow");
    }

    #[test]
    fn subfic_clears_xer_ca_on_borrow() {
        let mut s = PpuState::new();
        s.gpr[3] = 10;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Subfic {
                rt: 4,
                ra: 3,
                imm: 5,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 5u64.wrapping_sub(10));
        assert!(!s.xer_ca(), "subfic clears stale CA when borrow occurs");
    }

    #[test]
    fn addic_sets_xer_ca_on_carry() {
        let mut s = PpuState::new();
        s.gpr[3] = u64::MAX;
        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::Addic {
                rt: 4,
                ra: 3,
                imm: 1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0);
        assert!(s.xer_ca(), "addic sets CA when carry out");
    }

    #[test]
    fn addic_clears_xer_ca_on_no_carry() {
        let mut s = PpuState::new();
        s.gpr[3] = 1;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Addic {
                rt: 4,
                ra: 3,
                imm: 1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 2);
        assert!(!s.xer_ca(), "addic clears stale CA when no carry");
    }

    #[test]
    fn addic_negative_immediate_sign_extends_and_clears_ca() {
        // RA=0, imm=-1: the sign-extended -1 is 0xFFFF_FFFF_FFFF_FFFF.
        // 0 + (-1 sign-ext) wraps to 0xFFFF... with no carry out (the
        // unsigned add of 0 + 0xFFFF... is below 2^64).
        let mut s = PpuState::new();
        s.gpr[3] = 0;
        s.set_xer_ca(true);
        exec_no_mem(
            &PpuInstruction::Addic {
                rt: 4,
                ra: 3,
                imm: -1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0xFFFF_FFFF_FFFF_FFFF);
        assert!(!s.xer_ca(), "0 + (-1 sign-ext) does not generate carry");
    }

    #[test]
    fn cmpwi_propagates_xer_so_into_cr_field() {
        // SO is sticky once set; every subsequent compare must copy
        // it into the LSB of the CR field. A compare that drops SO
        // would leave guest branch logic looking at stale data.
        let mut s = PpuState::new();
        s.gpr[3] = 0;
        s.set_xer_ov(true); // sets both OV and SO
        exec_no_mem(
            &PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 0,
            },
            &mut s,
        );
        // EQ + SO: 0b0010 | 0b0001 = 0b0011.
        assert_eq!(s.cr_field(0), 0b0011);
    }

    #[test]
    fn cmpw_propagates_xer_so_with_lt_result() {
        let mut s = PpuState::new();
        s.gpr[3] = 1;
        s.gpr[4] = 2;
        s.set_xer_ov(true);
        exec_no_mem(
            &PpuInstruction::Cmpw {
                bf: 7,
                ra: 3,
                rb: 4,
            },
            &mut s,
        );
        // LT + SO: 0b1000 | 0b0001 = 0b1001.
        assert_eq!(s.cr_field(7), 0b1001);
    }

    #[test]
    fn andi_dot_propagates_xer_so_into_cr0() {
        // andi. routes through set_cr0_from_result, which OR-in's SO.
        // Hand-rolled CR0 construction that ignores XER[SO] would
        // produce a CR0 with the SO bit always zero.
        let mut s = PpuState::new();
        s.gpr[3] = 0xFF;
        s.set_xer_ov(true);
        exec_no_mem(
            &PpuInstruction::AndiDot {
                ra: 4,
                rs: 3,
                imm: 0x0F,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0x0F);
        // GT (positive non-zero) + SO: 0b0100 | 0b0001 = 0b0101.
        assert_eq!(s.cr_field(0), 0b0101);
    }

    #[test]
    fn andis_dot_shifts_immediate_left_16() {
        // andis. masks RS with (UI << 16). Reading andis. as andi.
        // would mask with 0x0F instead of 0x000F_0000 here.
        let mut s = PpuState::new();
        s.gpr[3] = 0x00FF_00FF;
        exec_no_mem(
            &PpuInstruction::AndisDot {
                ra: 4,
                rs: 3,
                imm: 0x0F,
            },
            &mut s,
        );
        // 0x00FF_00FF & 0x000F_0000 = 0x000F_0000.
        assert_eq!(s.gpr[4], 0x000F_0000);
        assert_eq!(s.cr_field(0), 0b0100); // GT (positive nonzero)
    }

    #[test]
    fn andis_dot_zero_result_sets_eq() {
        // No bit overlap between RS and (UI << 16) -> result 0 -> EQ.
        let mut s = PpuState::new();
        s.gpr[3] = 0x0000_FFFF; // bits 0..16 only
        exec_no_mem(
            &PpuInstruction::AndisDot {
                ra: 4,
                rs: 3,
                imm: 0x0F, // shifted to 0x000F_0000 -- no overlap
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0);
        assert_eq!(s.cr_field(0), 0b0010); // EQ
    }

    #[test]
    fn addic_dot_records_to_cr0_and_sets_ca() {
        // addic. carries on overflow exactly like addic AND
        // unconditionally writes CR0.
        let mut s = PpuState::new();
        s.gpr[3] = u64::MAX;
        s.set_xer_ca(false);
        exec_no_mem(
            &PpuInstruction::AddicDot {
                rt: 4,
                ra: 3,
                imm: 1,
            },
            &mut s,
        );
        assert_eq!(s.gpr[4], 0);
        assert!(s.xer_ca(), "addic. sets CA on carry out");
        assert_eq!(s.cr_field(0), 0b0010, "addic. records EQ for zero result");
    }

    #[test]
    fn mulhwu_cr0_treats_high_bit_result_as_positive() {
        // mulhwu produces a 32-bit unsigned high half stored
        // zero-extended in RT. CR0 must see the same value, i.e. a
        // result with the MSB of the low 32 set is GT, not LT.
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFFu32 as u64;
        s.gpr[4] = 0xFFFF_FFFFu32 as u64;
        exec_no_mem(
            &PpuInstruction::Mulhwu {
                rt: 5,
                ra: 3,
                rb: 4,
                rc: true,
            },
            &mut s,
        );
        // (0xFFFF_FFFF * 0xFFFF_FFFF) >> 32 = 0xFFFF_FFFE.
        assert_eq!(s.gpr[5], 0xFFFF_FFFE);
        assert_eq!(s.cr_field(0), 0b0100, "GT, not LT");
    }

    #[test]
    fn divwu_cr0_treats_high_bit_result_as_positive() {
        let mut s = PpuState::new();
        s.gpr[3] = 0xFFFF_FFFFu32 as u64;
        s.gpr[4] = 1;
        exec_no_mem(
            &PpuInstruction::Divwu {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: true,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0xFFFF_FFFF);
        assert_eq!(s.cr_field(0), 0b0100, "GT, not LT");
    }

    #[test]
    fn mtcrf_reads_low_32_bits_of_rs() {
        // Source register has different patterns in the high vs low 32
        // bits. The PPC spec says CR receives bits 32:63 (the low 32 in
        // little-endian Rust terms). A regression that reads the high
        // half would set CR to 0xAAAA_AAAA instead of 0x5555_5555.
        let mut s = PpuState::new();
        s.gpr[3] = 0xAAAA_AAAA_5555_5555;
        s.cr = 0;
        exec_no_mem(
            &PpuInstruction::Mtcrf {
                rs: 3,
                crm: 0xFF, // all 8 fields
            },
            &mut s,
        );
        assert_eq!(s.cr, 0x5555_5555, "mtcrf must take the low 32 bits of RS");
    }
}
