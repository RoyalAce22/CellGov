//! Typed PPU instruction forms.
//!
//! Variants carry decoded register indices, immediates, and flags.
//! Decode produces these; execute consumes them. Unknown encodings
//! decode to `PpuDecodeError::Unsupported` rather than a variant.

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
    Lwzu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
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
    /// Load doubleword with update (DS-form). `imm & 3 == 0`.
    Ldu {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load doubleword (DS-form). `imm & 3 == 0`.
    Ld {
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
    /// Store word with update. Requires `ra != 0 && ra != rs`.
    Stwu {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store doubleword with update. Requires `ra != 0 && ra != rs` and `imm & 3 == 0`.
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
    Sth {
        rs: u8,
        ra: u8,
        imm: i16,
    },
    /// Store doubleword (DS-form). `imm & 3 == 0`.
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
    Add {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Or {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Subf {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Subfc {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Subfe {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Neg {
        rt: u8,
        ra: u8,
    },
    Mullw {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Mulhwu {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Mulhw {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Mulhdu {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Mulhd {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Adde {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Addze {
        rt: u8,
        ra: u8,
    },
    Mulld {
        rt: u8,
        ra: u8,
        rb: u8,
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
    },
    Divwu {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Divd {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    Divdu {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    And {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Andc {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Nor {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Xor {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    AndiDot {
        ra: u8,
        rs: u8,
        imm: u16,
    },
    Slw {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Srw {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Srawi {
        ra: u8,
        rs: u8,
        sh: u8,
    },
    Sraw {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Srad {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Sradi {
        ra: u8,
        rs: u8,
        sh: u8,
    },
    Sld {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Srd {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Cntlzw {
        ra: u8,
        rs: u8,
    },
    Cntlzd {
        ra: u8,
        rs: u8,
    },
    Orc {
        ra: u8,
        rs: u8,
        rb: u8,
    },
    Extsh {
        ra: u8,
        rs: u8,
    },
    Extsb {
        ra: u8,
        rs: u8,
    },
    Extsw {
        ra: u8,
        rs: u8,
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
    },
    /// `ra` is both input and output: unmasked bits are preserved.
    Rlwimi {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        me: u8,
    },
    Rlwnm {
        ra: u8,
        rs: u8,
        rb: u8,
        mb: u8,
        me: u8,
    },
    /// `sh` and `mb` are 6-bit MD-form fields; mask covers `mb..=63`.
    Rldicl {
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
    },
    /// `sh` and `me` are 6-bit MD-form fields; mask covers `0..=me`.
    Rldicr {
        ra: u8,
        rs: u8,
        sh: u8,
        me: u8,
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

    /// Generic double-precision FP (primary 63). `xo` selects the op.
    Fp63 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
    },
    /// Generic single-precision FP (primary 59). `xo` selects the op.
    Fp59 {
        xo: u16,
        frt: u8,
        fra: u8,
        frb: u8,
        frc: u8,
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

    // -- System --
    /// System call. LV2 convention: syscall number in r11.
    Sc,
}

impl PpuInstruction {
    /// Variant name as a `&'static str`, without allocation.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Lwz { .. } => "Lwz",
            Self::Lbz { .. } => "Lbz",
            Self::Lhz { .. } => "Lhz",
            Self::Lha { .. } => "Lha",
            Self::Lwzu { .. } => "Lwzu",
            Self::Lbzu { .. } => "Lbzu",
            Self::Lhzu { .. } => "Lhzu",
            Self::Ldu { .. } => "Ldu",
            Self::Ld { .. } => "Ld",
            Self::Stw { .. } => "Stw",
            Self::Stwu { .. } => "Stwu",
            Self::Stdu { .. } => "Stdu",
            Self::Stb { .. } => "Stb",
            Self::Sth { .. } => "Sth",
            Self::Std { .. } => "Std",
            Self::Addi { .. } => "Addi",
            Self::Addis { .. } => "Addis",
            Self::Subfic { .. } => "Subfic",
            Self::Mulli { .. } => "Mulli",
            Self::Addic { .. } => "Addic",
            Self::Add { .. } => "Add",
            Self::Or { .. } => "Or",
            Self::Subf { .. } => "Subf",
            Self::Subfc { .. } => "Subfc",
            Self::Subfe { .. } => "Subfe",
            Self::Neg { .. } => "Neg",
            Self::Mullw { .. } => "Mullw",
            Self::Mulhwu { .. } => "Mulhwu",
            Self::Mulhw { .. } => "Mulhw",
            Self::Mulhdu { .. } => "Mulhdu",
            Self::Mulhd { .. } => "Mulhd",
            Self::Adde { .. } => "Adde",
            Self::Addze { .. } => "Addze",
            Self::Divw { .. } => "Divw",
            Self::Divwu { .. } => "Divwu",
            Self::Divd { .. } => "Divd",
            Self::Divdu { .. } => "Divdu",
            Self::Mulld { .. } => "Mulld",
            Self::Ldarx { .. } => "Ldarx",
            Self::Stdcx { .. } => "Stdcx",
            Self::Lwarx { .. } => "Lwarx",
            Self::Stwcx { .. } => "Stwcx",
            Self::Xori { .. } => "Xori",
            Self::Xoris { .. } => "Xoris",
            Self::And { .. } => "And",
            Self::Andc { .. } => "Andc",
            Self::Nor { .. } => "Nor",
            Self::Xor { .. } => "Xor",
            Self::AndiDot { .. } => "AndiDot",
            Self::Slw { .. } => "Slw",
            Self::Srw { .. } => "Srw",
            Self::Srawi { .. } => "Srawi",
            Self::Sraw { .. } => "Sraw",
            Self::Srad { .. } => "Srad",
            Self::Sradi { .. } => "Sradi",
            Self::Sld { .. } => "Sld",
            Self::Srd { .. } => "Srd",
            Self::Cntlzw { .. } => "Cntlzw",
            Self::Cntlzd { .. } => "Cntlzd",
            Self::Orc { .. } => "Orc",
            Self::Extsh { .. } => "Extsh",
            Self::Extsb { .. } => "Extsb",
            Self::Extsw { .. } => "Extsw",
            Self::Ori { .. } => "Ori",
            Self::Oris { .. } => "Oris",
            Self::Cmpwi { .. } => "Cmpwi",
            Self::Cmplwi { .. } => "Cmplwi",
            Self::Cmpw { .. } => "Cmpw",
            Self::Cmplw { .. } => "Cmplw",
            Self::Cmpd { .. } => "Cmpd",
            Self::Cmpld { .. } => "Cmpld",
            Self::B { .. } => "B",
            Self::Bc { .. } => "Bc",
            Self::Bclr { .. } => "Bclr",
            Self::Bcctr { .. } => "Bcctr",
            Self::Lwzx { .. } => "Lwzx",
            Self::Lbzx { .. } => "Lbzx",
            Self::Ldx { .. } => "Ldx",
            Self::Lhzx { .. } => "Lhzx",
            Self::Stwx { .. } => "Stwx",
            Self::Stdx { .. } => "Stdx",
            Self::Stdux { .. } => "Stdux",
            Self::Stbx { .. } => "Stbx",
            Self::Mftb { .. } => "Mftb",
            Self::Mfcr { .. } => "Mfcr",
            Self::Mtcrf { .. } => "Mtcrf",
            Self::Mflr { .. } => "Mflr",
            Self::Mtlr { .. } => "Mtlr",
            Self::Mfctr { .. } => "Mfctr",
            Self::Mtctr { .. } => "Mtctr",
            Self::Rlwinm { .. } => "Rlwinm",
            Self::Rlwimi { .. } => "Rlwimi",
            Self::Rlwnm { .. } => "Rlwnm",
            Self::Rldicl { .. } => "Rldicl",
            Self::Rldicr { .. } => "Rldicr",
            Self::Vx { .. } => "Vx",
            Self::Va { .. } => "Va",
            Self::Vxor { .. } => "Vxor",
            Self::Lvlx { .. } => "Lvlx",
            Self::Lvrx { .. } => "Lvrx",
            Self::Stvx { .. } => "Stvx",
            Self::Lfs { .. } => "Lfs",
            Self::Lfd { .. } => "Lfd",
            Self::Stfs { .. } => "Stfs",
            Self::Stfd { .. } => "Stfd",
            Self::Stfsu { .. } => "Stfsu",
            Self::Stfdu { .. } => "Stfdu",
            Self::Stfiwx { .. } => "Stfiwx",
            Self::Fp63 { .. } => "Fp63",
            Self::Fp59 { .. } => "Fp59",
            Self::Li { .. } => "Li",
            Self::Mr { .. } => "Mr",
            Self::Slwi { .. } => "Slwi",
            Self::Srwi { .. } => "Srwi",
            Self::Clrlwi { .. } => "Clrlwi",
            Self::Nop => "Nop",
            Self::CmpwZero { .. } => "CmpwZero",
            Self::Clrldi { .. } => "Clrldi",
            Self::Sldi { .. } => "Sldi",
            Self::Srdi { .. } => "Srdi",
            Self::LwzCmpwi { .. } => "LwzCmpwi",
            Self::LiStw { .. } => "LiStw",
            Self::MflrStw { .. } => "MflrStw",
            Self::LwzMtlr { .. } => "LwzMtlr",
            Self::MflrStd { .. } => "MflrStd",
            Self::LdMtlr { .. } => "LdMtlr",
            Self::StdStd { .. } => "StdStd",
            Self::CmpwiBc { .. } => "CmpwiBc",
            Self::CmpwBc { .. } => "CmpwBc",
            Self::Consumed => "Consumed",
            Self::Sc => "Sc",
        }
    }
}

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
            PpuInstruction::Sc,
            PpuInstruction::Fp63 {
                xo: 0,
                frt: 0,
                fra: 0,
                frb: 0,
                frc: 0,
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
        };
        let name: &'static str = insn.variant_name();
        assert_eq!(name, "Add");
    }
}
