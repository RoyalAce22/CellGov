//! Typed PPU instruction forms.
//!
//! Variants carry decoded register indices, immediates, and flags.
//! Decode produces these; execute consumes them. Unknown encodings
//! decode to `PpuDecodeError::Unsupported` rather than a variant.
//!
//! **DS-form immediates** (`Ld`, `Ldu`, `Std`, `Stdu`, `Lwa`) are
//! stored as byte offsets with the low 2 bits always zero, not the
//! raw 14-bit DS field. The decoder produces them via
//! `(raw & 0xFFFC) as i16`, which keeps the field shifted into bits
//! 2:15 and sign-extended via the `i16` representation. Executors
//! can consume `imm` directly as a signed byte offset; no further
//! shift is needed.

/// A decoded PPU instruction. Field names follow PPC ISA conventions
/// (`rt`/`rs`/`ra`/`rb`, `imm`, `offset`, `link`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PpuInstruction {
    // -- Integer loads --
    Lwz {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Lbz {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Lhz {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Lha {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load word zero with update. Requires `ra != 0 && ra != rt`.
    Lwzu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load byte zero with update. Requires `ra != 0 && ra != rt`.
    Lbzu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load halfword zero with update. Requires `ra != 0 && ra != rt`.
    Lhzu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load doubleword with update (DS-form). Requires
    /// `ra != 0 && ra != rt`; `imm` is a byte offset with low 2
    /// bits zero.
    Ldu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load doubleword (DS-form). `imm` is a byte offset with low
    /// 2 bits zero.
    Ld {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load word algebraic (DS-form, primary 58 sub=2). Sign-extends
    /// the 32-bit value into the 64-bit RT. `imm` is a byte offset
    /// with low 2 bits zero.
    Lwa {
        rt: u8,
        ra: u8,
        imm: i16,
    },

    // -- Integer stores --
    Stw {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store word with update. Requires `ra != 0`. (Unlike load
    /// with update, the ISA permits `rs == ra` here -- the store
    /// happens first, then EA is written to RA.)
    Stwu {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store doubleword with update (DS-form). Requires `ra != 0`;
    /// `rs == ra` is permitted (see `Stwu`). `imm` is a byte offset
    /// with low 2 bits zero.
    Stdu {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    Stb {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store byte with update. Requires `ra != 0`.
    Stbu {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    Sth {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store halfword with update. Requires `ra != 0`.
    Sthu {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store doubleword (DS-form). `imm` is a byte offset with low
    /// 2 bits zero.
    Std {
        rs: u8,
        ra: u8,
        imm: i16,
    },

    // -- Integer arithmetic / immediate --
    /// `ra == 0` means literal zero (not GPR0); this is how `li` is encoded.
    Addi {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// `ra == 0` means literal zero; this is how `lis` is encoded.
    Addis {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Subfic {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Mulli {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Addic {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// `addic.` (primary 13). Same arithmetic as `Addic` but
    /// always records to CR0; the ISA exposes the dot form as a
    /// distinct primary opcode rather than an Rc bit.
    AddicDot {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    Add {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Or {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Subf {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Subfc {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Subfe {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Neg {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    Mullw {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Mulhwu {
        rt: u8,
        ra: u8,
        rb: u8,
        rc: bool,
    },
    Mulhw {
        rt: u8,
        ra: u8,
        rb: u8,
        rc: bool,
    },
    Mulhdu {
        rt: u8,
        ra: u8,
        rb: u8,
        rc: bool,
    },
    Mulhd {
        rt: u8,
        ra: u8,
        rb: u8,
        rc: bool,
    },
    Adde {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Addze {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    Mulld {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    /// Load-doubleword-and-reserve. Under the single-threaded model
    /// this is equivalent to `Ldx`.
    Ldarx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Store-doubleword-conditional. Under the single-threaded model
    /// this always succeeds and sets CR0 EQ.
    Stdcx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    /// Load-word-and-reserve. Under the single-threaded model this
    /// is equivalent to `Lwzx`.
    Lwarx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Store-word-conditional. Under the single-threaded model this
    /// always succeeds and sets CR0 EQ.
    Stwcx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    Xori {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    Xoris {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    Divw {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Divwu {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Divd {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    Divdu {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    And {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Andc {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Nor {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Xor {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    AndiDot {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    /// `andis.` (primary 29). ANDs RS with `(imm as u32) << 16`
    /// and always records to CR0. The decoder stores the raw 16-bit
    /// UI; the executor handles the shift.
    AndisDot {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    Slw {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Srw {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Srawi {
        ra: u8,
        rs: u8,
        sh: u8,
        rc: bool,
    },
    Sraw {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Srad {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Sradi {
        ra: u8,
        rs: u8,
        sh: u8,
        rc: bool,
    },
    Sld {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Srd {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Cntlzw {
        ra: u8,
        rs: u8,
        rc: bool,
    },
    Cntlzd {
        ra: u8,
        rs: u8,
        rc: bool,
    },
    Orc {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    Extsh {
        ra: u8,
        rs: u8,
        rc: bool,
    },
    Extsb {
        ra: u8,
        rs: u8,
        rc: bool,
    },
    Extsw {
        ra: u8,
        rs: u8,
        rc: bool,
    },
    /// `imm == 0 && ra == rs` encodes `nop`.
    Ori {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    Oris {
        ra: u8,
        rs: u8,
        imm: u16,
    },

    // -- Compare --
    // `bf` is the CR field index (0..=7).
    Cmpwi {
        bf: u8,
        ra: u8,
        imm: i16,
    },
    Cmplwi {
        bf: u8,
        ra: u8,
        imm: u16,
    },
    /// 64-bit signed compare immediate (L=1 variant of primary 11).
    Cmpdi {
        bf: u8,
        ra: u8,
        imm: i16,
    },
    /// 64-bit unsigned compare immediate (L=1 variant of primary 10).
    Cmpldi {
        bf: u8,
        ra: u8,
        imm: u16,
    },
    Cmpw {
        bf: u8,
        ra: u8,
        rb: u8,
    },
    Cmplw {
        bf: u8,
        ra: u8,
        rb: u8,
    },
    Cmpd {
        bf: u8,
        ra: u8,
        rb: u8,
    },
    Cmpld {
        bf: u8,
        ra: u8,
        rb: u8,
    },

    // -- Branch --
    /// `offset` is the already-sign-extended 26-bit LI field in bytes.
    /// `aa` selects absolute (target = offset) vs relative (PC + offset).
    B {
        offset: i32,
        aa: bool,
        link: bool,
    },
    Bc {
        bo: u8,
        bi: u8,
        offset: i16,
        aa: bool,
        link: bool,
    },
    Bclr {
        bo: u8,
        bi: u8,
        link: bool,
    },
    Bcctr {
        bo: u8,
        bi: u8,
        link: bool,
    },

    // -- CR-logical (XL-form, opcode 19) --
    /// `mcrf BF, BFA`: copy 4-bit CR field `crfs` into field `crfd`.
    Mcrf {
        crfd: u8,
        crfs: u8,
    },
    /// `crand BT, BA, BB`: CR[bt] = CR[ba] AND CR[bb].
    Crand {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crandc BT, BA, BB`: CR[bt] = CR[ba] AND NOT CR[bb].
    Crandc {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `cror BT, BA, BB`: CR[bt] = CR[ba] OR CR[bb].
    Cror {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crorc BT, BA, BB`: CR[bt] = CR[ba] OR NOT CR[bb].
    Crorc {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crxor BT, BA, BB`: CR[bt] = CR[ba] XOR CR[bb].
    Crxor {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crnand BT, BA, BB`: CR[bt] = NOT (CR[ba] AND CR[bb]).
    Crnand {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crnor BT, BA, BB`: CR[bt] = NOT (CR[ba] OR CR[bb]).
    Crnor {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `creqv BT, BA, BB`: CR[bt] = NOT (CR[ba] XOR CR[bb]).
    Creqv {
        bt: u8,
        ba: u8,
        bb: u8,
    },

    // -- Indexed loads/stores --
    Lwzx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Lbzx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Ldx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Lhzx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Stwx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    Stdx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store-doubleword-with-update-indexed. Requires `ra != 0`.
    Stdux {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    Stbx {
        rs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Special-purpose register moves --
    /// Move-from-time-base. The model advances TB by 1 per read.
    Mftb {
        rt: u8,
    },
    /// Move-from-time-base-upper. Returns the upper 32 bits of TB.
    Mftbu {
        rt: u8,
    },
    Mfcr {
        rt: u8,
    },
    Mtcrf {
        rs: u8,
        crm: u8,
    },
    Mflr {
        rt: u8,
    },
    Mtlr {
        rs: u8,
    },
    Mfctr {
        rt: u8,
    },
    Mtctr {
        rs: u8,
    },

    // -- Rotate/shift (subset) --
    Rlwinm {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        me: u8,
        rc: bool,
    },
    /// `ra` is both input and output: unmasked bits are preserved.
    Rlwimi {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        me: u8,
        rc: bool,
    },
    Rlwnm {
        ra: u8,
        rs: u8,
        rb: u8,
        mb: u8,
        me: u8,
        rc: bool,
    },
    /// `sh` and `mb` are 6-bit MD-form fields; mask covers `mb..=63`.
    Rldicl {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        rc: bool,
    },
    /// `sh` and `me` are 6-bit MD-form fields; mask covers `0..=me`.
    Rldicr {
        ra: u8,
        rs: u8,
        sh: u8,
        me: u8,
        rc: bool,
    },
    /// Rotate left doubleword immediate then clear. Mask covers `mb..=(63-sh)`.
    Rldic {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        rc: bool,
    },
    /// Rotate left doubleword immediate then mask insert. Bits outside
    /// `mb..=(63-sh)` preserve the prior `ra` value.
    Rldimi {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        rc: bool,
    },

    // -- Vector (AltiVec / VMX) --
    /// Generic VX-form. `xo` is the 11-bit extended opcode; execution
    /// dispatches on it rather than opening a new variant per VMX op.
    Vx {
        xo: u16,
        vt: u8,
        va: u8,
        vb: u8,
    },
    /// Generic VA-form (four register operands, 6-bit sub-opcode).
    Va {
        xo: u8,
        vt: u8,
        va: u8,
        vb: u8,
        vc: u8,
    },
    /// Vector XOR. Also decodable as `Vx { xo: 0x4c4, .. }`.
    Vxor {
        vt: u8,
        va: u8,
        vb: u8,
    },
    /// Vector shift left double by octet immediate. `shb` is a 4-bit byte shift.
    Vsldoi {
        vt: u8,
        va: u8,
        vb: u8,
        shb: u8,
    },
    Lvlx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    Lvrx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    /// Store-vector-indexed. The effective address is aligned down to
    /// a 16-byte boundary before the store.
    Stvx {
        vs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Floating-point loads/stores --
    Lfs {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    Lfd {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    Stfs {
        frs: u8,
        ra: u8,
        imm: i16,
    },
    Stfd {
        frs: u8,
        ra: u8,
        imm: i16,
    },
    Stfsu {
        frs: u8,
        ra: u8,
        imm: i16,
    },
    Stfdu {
        frs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store-float-as-integer-word. The low 32 bits of `fpr[frs]` are
    /// written verbatim -- there is no float-to-int conversion.
    Stfiwx {
        frs: u8,
        ra: u8,
        rb: u8,
    },

    // -- X-form floating-point loads/stores (opcode 31) --
    /// `lfsx FRT, RA, RB`: load single, round to double in FRT.
    Lfsx {
        frt: u8,
        ra: u8,
        rb: u8,
    },
    /// `lfsux FRT, RA, RB`: lfsx with EA written back to RA (RA != 0).
    Lfsux {
        frt: u8,
        ra: u8,
        rb: u8,
    },
    /// `lfdx FRT, RA, RB`: load 64-bit double into FRT.
    Lfdx {
        frt: u8,
        ra: u8,
        rb: u8,
    },
    /// `lfdux FRT, RA, RB`: lfdx with EA written back to RA (RA != 0).
    Lfdux {
        frt: u8,
        ra: u8,
        rb: u8,
    },
    /// `stfsx FRS, RA, RB`: round FRS to single, store 32 bits.
    Stfsx {
        frs: u8,
        ra: u8,
        rb: u8,
    },
    /// `stfsux FRS, RA, RB`: stfsx with EA written back to RA (RA != 0).
    Stfsux {
        frs: u8,
        ra: u8,
        rb: u8,
    },
    /// `stfdx FRS, RA, RB`: store 64-bit double from FRS.
    Stfdx {
        frs: u8,
        ra: u8,
        rb: u8,
    },
    /// `stfdux FRS, RA, RB`: stfdx with EA written back to RA (RA != 0).
    Stfdux {
        frs: u8,
        ra: u8,
        rb: u8,
    },

    /// Generic double-precision FP (primary 63). `xo` selects the op.
    /// `rc` is preserved at decode but not yet honored by the executor
    /// (FPSCR/CR1 plumbing pending).
    Fp63 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
        rc: bool,
    },
    /// Generic single-precision FP (primary 59). `xo` selects the op.
    /// `rc` is preserved at decode but not yet honored by the executor
    /// (FPSCR/CR1 plumbing pending).
    Fp59 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
        rc: bool,
    },

    // -- Quickened (specialized) forms --
    Li {
        rt: u8,
        imm: i16,
    },
    Mr {
        ra: u8,
        rs: u8,
    },
    Slwi {
        ra: u8,
        rs: u8,
        n: u8,
    },
    Srwi {
        ra: u8,
        rs: u8,
        n: u8,
    },
    Clrlwi {
        ra: u8,
        rs: u8,
        n: u8,
    },
    Nop,
    /// Quickened form of `cmpwi crF, rA, 0`.
    CmpwZero {
        bf: u8,
        ra: u8,
    },
    Clrldi {
        ra: u8,
        rs: u8,
        n: u8,
    },
    Sldi {
        ra: u8,
        rs: u8,
        n: u8,
    },
    Srdi {
        ra: u8,
        rs: u8,
        n: u8,
    },

    // -- Superinstructions (compound 2-instruction pairs) --
    LwzCmpwi {
        rt: u8,
        ra_load: u8,
        offset: i16,
        bf: u8,
        cmp_imm: i16,
    },
    LiStw {
        rt: u8,
        imm: i16,
        ra_store: u8,
        store_offset: i16,
    },
    MflrStw {
        rt: u8,
        ra_store: u8,
        store_offset: i16,
    },
    LwzMtlr {
        rt: u8,
        ra_load: u8,
        offset: i16,
    },
    MflrStd {
        rt: u8,
        ra_store: u8,
        store_offset: i16,
    },
    LdMtlr {
        rt: u8,
        ra_load: u8,
        offset: i16,
    },
    /// Two adjacent `std` stores with `off2 == off1 + 8`.
    StdStd {
        rs1: u8,
        rs2: u8,
        ra: u8,
        offset1: i16,
    },
    CmpwiBc {
        bf: u8,
        ra: u8,
        imm: i16,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    CmpwBc {
        bf: u8,
        ra: u8,
        rb: u8,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    /// Placeholder for a slot absorbed by a preceding superinstruction.
    /// The fetch loop advances PC past this without executing.
    Consumed,

    // -- Cache block management --
    /// Data cache block set to zero (Book II Sec. 3.2.2). The 128-byte
    /// block containing `(RA|0)+(RB)` is written with zeros. No
    /// cache modelling is implied; under the deterministic model the
    /// visible effect is a 128-byte zero store at the aligned EA.
    Dcbz {
        ra: u8,
        rb: u8,
    },

    // -- System --
    /// System call. LV2 convention: syscall number in r11. The 7-bit
    /// LEV field selects the privilege level: PS3 usermode always
    /// issues LEV=0 (kernel syscall); LEV=1 would target an LV1
    /// hypercall. Preserved at decode so the executor can route on
    /// it when hypercall dispatch is wired.
    Sc {
        lev: u8,
    },
}

mod display;

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuDecodeError {
    /// No matching encoding for this 32-bit word.
    Unsupported(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_name_matches_debug_prefix() {
        let cases: &[PpuInstruction] = &[
            PpuInstruction::Addi {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::Lwz {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::B {
                offset: 0,
                aa: false,
                link: false,
            },
            PpuInstruction::Sc { lev: 0 },
            PpuInstruction::Fp63 {
                xo: 0,
                frt: 0,
                fra: 0,
                frb: 0,
                frc: 0,
                rc: false,
            },
            PpuInstruction::Consumed,
            PpuInstruction::Lwa {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::AddicDot {
                rt: 0,
                ra: 0,
                imm: 0,
            },
            PpuInstruction::AndisDot {
                ra: 0,
                rs: 0,
                imm: 0,
            },
        ];
        for insn in cases {
            let debug = format!("{insn:?}");
            let prefix = debug
                .split_once([' ', '{'])
                .map(|(n, _)| n)
                .unwrap_or(&debug);
            assert_eq!(
                insn.variant_name(),
                prefix,
                "variant_name mismatch for {debug}"
            );
        }
    }

    #[test]
    fn variant_name_is_static() {
        let insn = PpuInstruction::Add {
            rt: 3,
            ra: 4,
            rb: 5,
            oe: false,
            rc: false,
        };
        let name: &'static str = insn.variant_name();
        assert_eq!(name, "Add");
    }
}
