//! The reserved-field mask of each instruction.

use crate::instruction::PpuInstruction;

/// Bits of a word carrying `insn` that the decoder does not read.
///
/// Two words that differ only there decode to the same `insn`. A
/// field the form reserves but the decoder stores, such as the FRA
/// slot of `fsqrt` or the vA slot of a unary VX op, is read and so
/// is not named here.
///
/// [PPC-Book1 p:3 s:1.5.2 Reserved Fields and Reserved Values] a reserved field is ignored on read.
pub fn reserved_bits(insn: &PpuInstruction) -> u32 {
    use PpuInstruction as I;
    const RC: u32 = 0x0000_0001;
    const OE: u32 = 0x0000_0400;
    const RB: u32 = 0x0000_F800;
    const RA: u32 = 0x001F_0000;
    const RT: u32 = 0x03E0_0000;
    // PPC bit 9 of a compare, between BF and L.
    const CMP_BIT_9: u32 = 0x0040_0000;
    match *insn {
        I::Cmpwi { .. } | I::Cmpdi { .. } | I::Cmplwi { .. } | I::Cmpldi { .. } => CMP_BIT_9,
        I::Cmpw { .. } | I::Cmpd { .. } | I::Cmplw { .. } | I::Cmpld { .. } => CMP_BIT_9 | RC,
        // [PPC-Book1 p:8 s:1.7.3 SC-Form] every field but LEV and the marker bit is reserved.
        I::Sc { .. } => 0x03FF_F01C,
        // [PPC-Book2 p:26 s:3.3.3] sync reads L alone; bits 6:8, 11:20 and 31 are reserved.
        I::Sync { .. } => 0x039F_F801,
        // [PPC-Book2 p:28 s:3.3.3], [PPC-Book2 p:22 s:3.3.1] eieio and isync read no operand field.
        I::Eieio | I::Isync => 0x03FF_F801,
        // [PPC-Book1 p:30 s:2.4.4] mcrf reads BF and BFA only.
        I::Mcrf { .. } => 0x0063_F801,
        // [PPC-Book1 p:25 s:2.4.1] the BH hint and its reserved neighbours are not modelled.
        I::Bclr { .. } | I::Bcctr { .. } => RB,
        I::Crnor { .. }
        | I::Crandc { .. }
        | I::Crxor { .. }
        | I::Crnand { .. }
        | I::Crand { .. }
        | I::Creqv { .. }
        | I::Crorc { .. }
        | I::Cror { .. } => RC,
        I::Neg { .. } | I::Addze { .. } | I::Subfze { .. } | I::Subfme { .. } | I::Addme { .. } => {
            RB
        }
        I::Mulhwu { .. } | I::Mulhdu { .. } | I::Mulhd { .. } | I::Mulhw { .. } => OE,
        I::Cntlzw { .. }
        | I::Cntlzd { .. }
        | I::Extsh { .. }
        | I::Extsb { .. }
        | I::Extsw { .. } => RB,
        I::Popcntb { .. } => RB | RC,
        I::Tw { .. } | I::Td { .. } => RC,
        // [PPC-Book1 p:135 s:6.1] mcrxr reads BF only.
        I::Mcrxr { .. } => 0x0060_0000 | RA | RB | RC,
        I::Dcbz { .. } => RT | RC,
        I::Mfcr { .. } => 0x000F_F800 | RC,
        I::Mfocrf { .. } | I::Mtcrf { .. } | I::Mtocrf { .. } => 0x0000_0800 | RC,
        I::Lwzx { .. }
        | I::Lbzx { .. }
        | I::Ldx { .. }
        | I::Lhzx { .. }
        | I::Lwzux { .. }
        | I::Lbzux { .. }
        | I::Lhzux { .. }
        | I::Ldux { .. }
        | I::Lhax { .. }
        | I::Lhaux { .. }
        | I::Lwax { .. }
        | I::Lwaux { .. }
        | I::Sthx { .. }
        | I::Sthux { .. }
        | I::Stwux { .. }
        | I::Stbux { .. }
        | I::Lswi { .. }
        | I::Stswi { .. }
        | I::Lswx { .. }
        | I::Stswx { .. }
        | I::Ldarx { .. }
        | I::Lwarx { .. }
        | I::Stwx { .. }
        | I::Stdx { .. }
        | I::Stdux { .. }
        | I::Stbx { .. }
        | I::Ldbrx { .. }
        | I::Lwbrx { .. }
        | I::Sdbrx { .. }
        | I::Stwbrx { .. }
        | I::Lhbrx { .. }
        | I::Sthbrx { .. }
        | I::Stfiwx { .. }
        | I::Lfsx { .. }
        | I::Lfsux { .. }
        | I::Lfdx { .. }
        | I::Lfdux { .. }
        | I::Stfsx { .. }
        | I::Stfsux { .. }
        | I::Stfdx { .. }
        | I::Stfdux { .. }
        | I::Lvlx { .. }
        | I::Lvrx { .. }
        | I::Lvlxl { .. }
        | I::Lvrxl { .. }
        | I::Stvlx { .. }
        | I::Stvrx { .. }
        | I::Stvlxl { .. }
        | I::Stvrxl { .. }
        | I::Lvsl { .. }
        | I::Lvebx { .. }
        | I::Lvsr { .. }
        | I::Lvehx { .. }
        | I::Lvewx { .. }
        | I::Lvx { .. }
        | I::Stvebx { .. }
        | I::Stvehx { .. }
        | I::Stvewx { .. }
        | I::Lvxl { .. }
        | I::Stvx { .. }
        | I::Stvxl { .. }
        | I::Mfxer { .. }
        | I::Mflr { .. }
        | I::Mfctr { .. }
        | I::Mfvrsave { .. }
        | I::Mftb { .. }
        | I::Mftbu { .. }
        | I::Mtxer { .. }
        | I::Mtlr { .. }
        | I::Mtctr { .. }
        | I::Mtvrsave { .. } => RC,
        // [AltiVec-PEM p:6-136 s:6.2] vsldoi's SHB is four bits; bit 21 is reserved.
        I::Vsldoi { .. } => 0x0000_0400,
        I::Lwz { .. }
        | I::Lbz { .. }
        | I::Lhz { .. }
        | I::Lha { .. }
        | I::Lhau { .. }
        | I::Lmw { .. }
        | I::Lwzu { .. }
        | I::Lbzu { .. }
        | I::Lhzu { .. }
        | I::Ldu { .. }
        | I::Ld { .. }
        | I::Lwa { .. }
        | I::Stw { .. }
        | I::Stwu { .. }
        | I::Stdu { .. }
        | I::Stb { .. }
        | I::Stbu { .. }
        | I::Stmw { .. }
        | I::Sth { .. }
        | I::Sthu { .. }
        | I::Std { .. }
        | I::Addi { .. }
        | I::Addis { .. }
        | I::Subfic { .. }
        | I::Mulli { .. }
        | I::Addic { .. }
        | I::AddicDot { .. }
        | I::Add { .. }
        | I::Or { .. }
        | I::Subf { .. }
        | I::Subfc { .. }
        | I::Subfe { .. }
        | I::Mullw { .. }
        | I::Adde { .. }
        | I::Mulld { .. }
        | I::Stdcx { .. }
        | I::Stwcx { .. }
        | I::Xori { .. }
        | I::Xoris { .. }
        | I::Divw { .. }
        | I::Divwu { .. }
        | I::Divd { .. }
        | I::Divdu { .. }
        | I::And { .. }
        | I::Andc { .. }
        | I::Nor { .. }
        | I::Xor { .. }
        | I::Eqv { .. }
        | I::Nand { .. }
        | I::AndiDot { .. }
        | I::AndisDot { .. }
        | I::Slw { .. }
        | I::Srw { .. }
        | I::Srawi { .. }
        | I::Sraw { .. }
        | I::Srad { .. }
        | I::Sradi { .. }
        | I::Sld { .. }
        | I::Srd { .. }
        | I::Orc { .. }
        | I::Ori { .. }
        | I::Oris { .. }
        | I::B { .. }
        | I::Bc { .. }
        | I::Rlwinm { .. }
        | I::Rlwimi { .. }
        | I::Rlwnm { .. }
        | I::Rldicl { .. }
        | I::Rldicr { .. }
        | I::Rldic { .. }
        | I::Rldimi { .. }
        | I::Rldcl { .. }
        | I::Rldcr { .. }
        | I::Vx { .. }
        | I::Va { .. }
        | I::Vxor { .. }
        | I::Lfs { .. }
        | I::Lfsu { .. }
        | I::Lfd { .. }
        | I::Lfdu { .. }
        | I::Stfs { .. }
        | I::Stfd { .. }
        | I::Stfsu { .. }
        | I::Stfdu { .. }
        | I::Fp63 { .. }
        | I::Fp59 { .. }
        | I::Li { .. }
        | I::Mr { .. }
        | I::Slwi { .. }
        | I::Srwi { .. }
        | I::Clrlwi { .. }
        | I::Nop
        | I::CmpwZero { .. }
        | I::Clrldi { .. }
        | I::Sldi { .. }
        | I::Srdi { .. }
        | I::LwzCmpwi { .. }
        | I::LiStw { .. }
        | I::MflrStw { .. }
        | I::LwzMtlr { .. }
        | I::MflrStd { .. }
        | I::LdMtlr { .. }
        | I::StdStd { .. }
        | I::CmpwiBc { .. }
        | I::CmpwBc { .. }
        | I::Consumed => 0,
    }
}
