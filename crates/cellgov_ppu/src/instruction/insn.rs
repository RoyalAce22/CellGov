//! [`PpuInstruction`] variants.
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
///
/// `IntoStaticStr` produces the variant identifier as a `&'static str`
/// via `<&'static str>::from(&insn)`. Consumers
/// (e.g. `PpuExecutionUnit::profile_prev`) store the name in
/// `&'static str` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr)]
#[allow(missing_docs)]
pub enum PpuInstruction {
    // -- Integer loads --
    // [PPC-Book1 p:34 s:3.3 Fixed-Point Load Instructions] lbz/lhz; lwz at p:37; ld/ldu/lwa at p:38-39.
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
    /// Load halfword algebraic with update. Requires `ra != 0 && ra != rt`.
    // [PPC-Book1 p:36 s:3.3 Fixed-Point Load Instructions] lhau D-form.
    Lhau {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load multiple word. Loads `32 - rt` words starting at EA into
    /// `gpr[rt..=31]`, each zero-extended into 64 bits.
    // [PPC-Book1 p:54 s:3.3 Fixed-Point Load Multiple Instruction] lmw D-form; EA word-aligned.
    Lmw {
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
    // [PPC-Book1 p:40 s:3.3 Fixed-Point Store Instructions] stb/stbu at p:40; sth/sthu at p:41; stw/stwu at p:42; std/stdu at p:43.
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
    /// Store multiple word. Stores the low 32 bits of
    /// `gpr[rs..=31]` into successive words at EA.
    // [PPC-Book1 p:54 s:3.3 Fixed-Point Store Multiple Instruction] stmw D-form; EA word-aligned.
    Stmw {
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
    // [PPC-Book1 p:51 s:3.3.8 Fixed-Point Arithmetic Instructions] addi/addis (ra==0 means 0).
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
    // [PPC-Book1 p:53 s:3.3.8 Fixed-Point Arithmetic Instructions] subfic.
    Subfic {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    // [PPC-Book1 p:56 s:3.3.9 Fixed-Point Multiply Instructions] mulli D-form.
    Mulli {
        rt: u8,
        ra: u8,
        imm: i16,
    },
    // [PPC-Book1 p:52 s:3.3.8 Fixed-Point Arithmetic Instructions] addic / addic. (primary 12 / 13).
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
    // [PPC-Book1 p:52 s:3.3.8 Fixed-Point Arithmetic Instructions] add XO-form.
    Add {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:67 s:3.3.13 Fixed-Point Logical Instructions] or X-form.
    Or {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    // [PPC-Book1 p:52 s:3.3.8 Fixed-Point Arithmetic Instructions] subf XO-form.
    Subf {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:53 s:3.3.8 Fixed-Point Arithmetic Instructions] subfc.
    Subfc {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:54 s:3.3.8 Fixed-Point Arithmetic Instructions] subfe (Subtract From Extended).
    Subfe {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:55 s:3.3.8 Fixed-Point Arithmetic Instructions] neg XO-form.
    Neg {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:56 s:3.3.9 Fixed-Point Multiply Instructions] mullw / mulld; mulhw* / mulhd* at p:57.
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
    // [PPC-Book1 p:54 s:3.3.8 Fixed-Point Arithmetic Instructions] adde XO-form.
    Adde {
        rt: u8,
        ra: u8,
        rb: u8,
        oe: bool,
        rc: bool,
    },
    // [PPC-Book1 p:55 s:3.3.8 Fixed-Point Arithmetic Instructions] addze (Add to Zero Extended).
    Addze {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    /// Subtract from zero extended: `RT = ~RA + CA`. Sets CA out.
    // [PPC-Book1 p:55 s:3.3.8] subfze XO-form.
    Subfze {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    /// Subtract from minus one extended: `RT = ~RA + CA + (-1)`.
    // [PPC-Book1 p:55 s:3.3.8] subfme XO-form.
    Subfme {
        rt: u8,
        ra: u8,
        oe: bool,
        rc: bool,
    },
    /// Add to minus one extended: `RT = RA + CA + (-1)`.
    // [PPC-Book1 p:55 s:3.3.8] addme XO-form.
    Addme {
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
    // [PPC-Book2 p:24 s:3.3 Atomic Update Primitives] lwarx / ldarx X-form; stwcx. / stdcx. at p:25.
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
    // [PPC-Book1 p:66 s:3.3.13 Fixed-Point Logical Instructions] xori / xoris D-form.
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
    // [PPC-Book1 p:58 s:3.3.10 Fixed-Point Divide Instructions] divw / divd; unsigned variants at p:59.
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
    // [PPC-Book1 p:67 s:3.3.13 Fixed-Point Logical Instructions] and / andc / nor / xor / orc X-form (p:67-68).
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
    /// Equivalent (XNOR): `RA = ~(RS XOR RB)`.
    // [PPC-Book1 p:65 s:3.3.13 Fixed-Point Logical Instructions] eqv X-form.
    Eqv {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    /// NAND: `RA = ~(RS & RB)`.
    // [PPC-Book1 p:65 s:3.3.13 Fixed-Point Logical Instructions] nand X-form.
    Nand {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    // [PPC-Book1 p:65 s:3.3.13 Fixed-Point Logical Instructions] andi. / andis. D-form (always record).
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
    // [PPC-Book1 p:77 s:3.3.14 Fixed-Point Shift Instructions] slw / sld X-form (sld also p:77).
    Slw {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    // [PPC-Book1 p:78 s:3.3.14 Fixed-Point Shift Instructions] srw / srd X-form.
    Srw {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    // [PPC-Book1 p:79 s:3.3.14 Fixed-Point Shift Instructions] srawi / sradi (immediate forms).
    Srawi {
        ra: u8,
        rs: u8,
        sh: u8,
        rc: bool,
    },
    // [PPC-Book1 p:80 s:3.3.14 Fixed-Point Shift Instructions] sraw / srad X-form.
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
    // [PPC-Book1 p:70 s:3.3.13 Fixed-Point Logical Instructions] cntlzw / cntlzd X-form.
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
    // [PPC-Book1 p:70 s:3.3.13 Fixed-Point Logical Instructions] popcntb X-form, primary 31 XO 122; no Rc.
    /// Population Count Bytes. For each of the 8 bytes of RS,
    /// count the number of 1-bits and place the count
    /// (0..=8) into the corresponding byte of RA.
    Popcntb {
        ra: u8,
        rs: u8,
    },
    // [PPC-Book1 p:64 s:3.3.10 Fixed-Point Trap Instructions] tw / td X-form, primary 31 XO 4 / 68; TO field selects up to five conditions.
    /// Trap Word. Compares `RA[32:63]` against `RB[32:63]` as
    /// signed 32-bit values; invokes the trap handler if any
    /// TO-selected condition holds, otherwise advances.
    Tw {
        to: u8,
        ra: u8,
        rb: u8,
    },
    /// Trap Doubleword. Same semantics as [`PpuInstruction::Tw`]
    /// but on the full 64-bit register contents.
    Td {
        to: u8,
        ra: u8,
        rb: u8,
    },
    // [PPC-Book1 p:135 s:6.1 Move To Condition Register from XER] mcrxr X-form, primary 31 XO 512; phased out of the architecture.
    /// Move to Condition Register from XER. Copies `XER[32:35]`
    /// (SO, OV, CA, reserved) into CR field `BF`, then zeroes
    /// `XER[32:35]`. Phased out in later ISAs but defined for
    /// PPC v2.02 (Cell PPU).
    Mcrxr {
        bf: u8,
    },
    // [PPC-Book1 p:68 s:3.3.13 Fixed-Point Logical Instructions] orc / nand / equivalent X-form.
    Orc {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
    },
    // [PPC-Book1 p:69 s:3.3.13 Fixed-Point Logical Instructions] extsb / extsh / extsw X-form.
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
    // [PPC-Book1 p:66 s:3.3.13 Fixed-Point Logical Instructions] ori / oris D-form.
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
    // [PPC-Book1 p:60 s:3.3.11 Fixed-Point Compare Instructions] cmpi/cmpwi (D); cmplwi/cmpldi at p:61; cmp/cmpw/cmpd X-form at p:60-61.
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
    // [PPC-Book1 p:24 s:2.4.1 Branch Instructions] b I-form / bc B-form; bclr / bcctr XL-form at p:25.
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
    // [PPC-Book1 p:30 s:2.4.3 Condition Register Logical Instructions] mcrf XL-form (move CR field).
    /// `mcrf BF, BFA`: copy 4-bit CR field `crfs` into field `crfd`.
    Mcrf {
        crfd: u8,
        crfs: u8,
    },
    // [PPC-Book1 p:28 s:2.4.3 Condition Register Logical Instructions] crand / cror / crxor / crnand XL-form; crnor / creqv / crandc / crorc at p:29.
    /// `crand BT, BA, BB`: `CR[bt] = CR[ba] AND CR[bb]`.
    Crand {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crandc BT, BA, BB`: `CR[bt] = CR[ba] AND NOT CR[bb]`.
    Crandc {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `cror BT, BA, BB`: `CR[bt] = CR[ba] OR CR[bb]`.
    Cror {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crorc BT, BA, BB`: `CR[bt] = CR[ba] OR NOT CR[bb]`.
    Crorc {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crxor BT, BA, BB`: `CR[bt] = CR[ba] XOR CR[bb]`.
    Crxor {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crnand BT, BA, BB`: `CR[bt] = NOT (CR[ba] AND CR[bb])`.
    Crnand {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `crnor BT, BA, BB`: `CR[bt] = NOT (CR[ba] OR CR[bb])`.
    Crnor {
        bt: u8,
        ba: u8,
        bb: u8,
    },
    /// `creqv BT, BA, BB`: `CR[bt] = NOT (CR[ba] XOR CR[bb])`.
    Creqv {
        bt: u8,
        ba: u8,
        bb: u8,
    },

    // -- Indexed loads/stores --
    // [PPC-Book1 p:34 s:3.3 Fixed-Point Load Instructions] X-form indexed loads (lbzx p:34, lhzx p:35, lwzx p:37, ldx p:39); indexed stores at p:40-43.
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

    // -- Indexed loads with update (X-form) --
    // [PPC-Book1 p:34 s:3.3.1 Fixed-Point Load Instructions]
    // Update forms write EA back to RA; require RA != 0 and RA != RT.
    /// Load word zero indexed with update.
    Lwzux {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load byte zero indexed with update.
    Lbzux {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load halfword zero indexed with update.
    Lhzux {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load doubleword indexed with update.
    Ldux {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load halfword algebraic indexed; sign-extends to RT.
    Lhax {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load halfword algebraic indexed with update.
    Lhaux {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load word algebraic indexed; sign-extends to RT.
    Lwax {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load word algebraic indexed with update.
    Lwaux {
        rt: u8,
        ra: u8,
        rb: u8,
    },

    // -- Indexed stores (X-form) --
    // [PPC-Book1 p:42 s:3.3.3 Fixed-Point Store Instructions] sthx p:41; sthux p:41; stwux p:42; stbux p:40.
    /// Store halfword indexed.
    Sthx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store halfword indexed with update.
    Sthux {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store word indexed with update.
    Stwux {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store byte indexed with update.
    Stbux {
        rs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Load / store string (X-form, primary 31) --
    // [PPC-Book1 p:55 s:3.3.5 Fixed-Point Load and Store String Instructions]
    // String moves transfer N bytes between memory and an aligned
    // sequence of GPRs, packed left-justified four bytes per
    // register and wrapping at r31 -> r0.
    /// Load String Word Immediate. `nb` is 1..32 with 0 encoding 32.
    Lswi {
        rt: u8,
        ra: u8,
        nb: u8,
    },
    /// Load String Word Indexed. Byte count comes from `xer[57:63]`.
    Lswx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    /// Store String Word Immediate. `nb` is 1..32 with 0 encoding 32.
    Stswi {
        rs: u8,
        ra: u8,
        nb: u8,
    },
    /// Store String Word Indexed. Byte count comes from `xer[57:63]`.
    Stswx {
        rs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Byte-reverse indexed loads and stores --
    // [PPC-Book1 p:51 s:3.3.4 Byte-Reverse Instructions] ldbrx X-form: little-endian 8-byte load to RT.
    Ldbrx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    // [PPC-Book1 p:50 s:3.3.4 Byte-Reverse Instructions] lwbrx X-form: little-endian 4-byte load, zero-extended to RT.
    Lwbrx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    // [PPC-Book1 p:50 s:3.3.4 Byte-Reverse Instructions] lhbrx X-form: little-endian 2-byte load, zero-extended to RT.
    Lhbrx {
        rt: u8,
        ra: u8,
        rb: u8,
    },
    // [CBE-Handbook p:734 s:A.2.1] sdbrx (Cell PPE name; the spec also writes `stdbrx`): little-endian 8-byte store from RS low 64.
    /// Store Doubleword Byte Reverse Indexed. The Handbook's
    /// definition page uses the `sdbrx` spelling; the same encoding
    /// is also spelled `stdbrx` elsewhere in upstream PPC literature.
    Sdbrx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    // [PPC-Book1 p:51 s:3.3.4 Byte-Reverse Instructions] stwbrx X-form: little-endian 4-byte store from RS low 32.
    Stwbrx {
        rs: u8,
        ra: u8,
        rb: u8,
    },
    // [PPC-Book1 p:51 s:3.3.4 Byte-Reverse Instructions] sthbrx X-form: little-endian 2-byte store from RS low 16.
    Sthbrx {
        rs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Special-purpose register moves --
    // [PPC-Book2 p:30 s:6.2 Reading the Time Base] mftb XFX-form; SPR encoding TBR=268 (TB), 269 (TBU).
    /// Move-from-time-base. The model advances TB by 1 per read.
    Mftb {
        rt: u8,
    },
    /// Move-from-time-base-upper. Returns the upper 32 bits of TB.
    Mftbu {
        rt: u8,
    },
    // [PPC-Book1 p:83 s:3.3.16 Move To/From System Register Instructions] mfcr XFX-form; mtcrf at p:83.
    Mfcr {
        rt: u8,
    },
    Mtcrf {
        rs: u8,
        crm: u8,
    },
    // [CBE-Handbook p:738 s:A.2.3.1 Book I Optional Instructions Implemented] mfocrf / mtocrf share XOs 19 / 144 with mfcr / mtcrf; bit 11 = 1 selects the one-field form. Cell PPE implements both.
    /// Move From One Condition Register Field. `crm` is one-hot; the
    /// set bit selects which CR field gets copied into the
    /// corresponding 4 bits of RT (other bits boundedly undefined per
    /// spec; CellGov produces the same nibble layout as mfcr for the
    /// selected field and zero elsewhere).
    Mfocrf {
        rt: u8,
        crm: u8,
    },
    /// Move To One Condition Register Field. `crm` is one-hot; the
    /// set bit selects which CR field gets RS[32+4*field..+4] copied
    /// in. Non-one-hot `crm` is boundedly undefined per spec; CellGov
    /// applies the mtcrf semantic (mask-each-set-bit) for that case.
    Mtocrf {
        rs: u8,
        crm: u8,
    },
    // [PPC-Book1 p:81 s:3.3.16 Move To/From System Register Instructions] mtspr (mtlr/mtctr extended); mfspr (mflr/mfctr) at p:82.
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
    // [PPC-Book1 p:42 s:3.2.2 Fixed-Point Exception Register] XER is SPR 1, problem-state read/write via mfspr/mtspr.
    /// Move From XER (`mfspr rT, 1`).
    Mfxer {
        rt: u8,
    },
    /// Move To XER (`mtspr 1, rS`).
    Mtxer {
        rs: u8,
    },
    // [AltiVec-PEM p:48 s:2.3.2 VRSAVE Register] VRSAVE is SPR 256, 32-bit, problem-state read/write via mfspr/mtspr.
    /// Move From VRSAVE (`mfspr rT, 256`).
    Mfvrsave {
        rt: u8,
    },
    /// Move To VRSAVE (`mtspr 256, rS`).
    Mtvrsave {
        rs: u8,
    },

    // -- Rotate/shift (subset) --
    // [PPC-Book1 p:73 s:3.3.12.1 Fixed-Point Rotate Instructions] rlwinm / rlwnm M-form (32-bit rotate w/ mask); rlwimi at p:72.
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
    // [PPC-Book1 p:72 s:3.3.12.1 Fixed-Point Rotate Instructions] rldicl / rldicr / rldic / rldimi MD-form (64-bit rotate w/ mask).
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
    /// Rotate left doubleword by RB (low 6 bits) then clear left of
    /// `mb`. MDS-form.
    // [PPC-Book1 p:75 s:3.3.12 Fixed-Point Rotate and Shift Instructions] rldcl MDS-form.
    Rldcl {
        ra: u8,
        rs: u8,
        rb: u8,
        mb: u8,
        rc: bool,
    },
    /// Rotate left doubleword by RB (low 6 bits) then clear right of
    /// `me`. MDS-form.
    // [PPC-Book1 p:75 s:3.3.12 Fixed-Point Rotate and Shift Instructions] rldcr MDS-form.
    Rldcr {
        ra: u8,
        rs: u8,
        rb: u8,
        me: u8,
        rc: bool,
    },

    // -- Vector (AltiVec / VMX) --
    // [AltiVec-PEM p:2] AltiVec architectural overview; VX/VA-form encoding under primary opcode 4.
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
    // [AltiVec-PEM p:6-177 s:6.2 AltiVec Instruction Set] vxor VX-form (XO=1220 / 0x4c4).
    /// Vector XOR. Also decodable as `Vx { xo: 0x4c4, .. }`.
    Vxor {
        vt: u8,
        va: u8,
        vb: u8,
    },
    // [AltiVec-PEM p:6-136 s:6.2 AltiVec Instruction Set] vsldoi VA-form (Shift Left Double by Octet Immediate, 4-bit SHB).
    /// Vector shift left double by octet immediate. `shb` is a 4-bit byte shift.
    Vsldoi {
        vt: u8,
        va: u8,
        vb: u8,
        shb: u8,
    },
    // [CBE-Handbook p:744] lvlx / lvrx Cell-specific VXU misaligned vector load (left/right indexed).
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
    /// Load Vector Left Indexed Last. Architecturally identical to
    /// [`PpuInstruction::Lvlx`]; the "Last" hint is a cache LRU
    /// directive only and CellGov has no cache model.
    // [CBE-Handbook p:744 s:A.3.3] lvlxl Cell-specific VXU misaligned vector load with LRU hint.
    Lvlxl {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    /// Load Vector Right Indexed Last. Architecturally identical to
    /// [`PpuInstruction::Lvrx`].
    // [CBE-Handbook p:744 s:A.3.3] lvrxl Cell-specific VXU misaligned vector load with LRU hint.
    Lvrxl {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    /// Store Vector Left Indexed. Partial vector store; bytes
    /// `[0..16-EA_mod16]` of `vr[vs]` are written to `MEM(EA, 16-m)`.
    // [CBE-Handbook p:744 s:A.3.3] stvlx Cell-specific VXU misaligned vector store.
    Stvlx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store Vector Right Indexed. Partial vector store; bytes
    /// `[16-m..16]` of `vr[vs]` are written to `MEM(aligned, m)`.
    // [CBE-Handbook p:744 s:A.3.3] stvrx Cell-specific VXU misaligned vector store.
    Stvrx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store Vector Left Indexed Last. Architecturally identical to
    /// [`PpuInstruction::Stvlx`].
    // [CBE-Handbook p:744 s:A.3.3] stvlxl with LRU hint.
    Stvlxl {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    /// Store Vector Right Indexed Last. Architecturally identical to
    /// [`PpuInstruction::Stvrx`].
    // [CBE-Handbook p:744 s:A.3.3] stvrxl with LRU hint.
    Stvrxl {
        vs: u8,
        ra: u8,
        rb: u8,
    },

    // -- AltiVec-memory family (X-form, primary 31) --
    // [AltiVec-PEM p:6-21 s:6.2] lvsl XO=6: shift-left permute control vector from EA low 4 bits.
    /// Load Vector for Shift Left. Builds a 16-byte permute-control
    /// vector from the low 4 bits of EA = (RA|0) + RB.
    Lvsl {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-15 s:6.2] lvebx XO=7: single-byte element load to byte EA mod 16 of vT.
    /// Load Vector Element Byte Indexed.
    Lvebx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-22 s:6.2] lvsr XO=38: shift-right permute control vector from EA low 4 bits.
    /// Load Vector for Shift Right. Builds a 16-byte permute-control
    /// vector from the low 4 bits of EA = (RA|0) + RB.
    Lvsr {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-16 s:6.2] lvehx XO=39: halfword element load to halfword EA mod 16 of vT.
    /// Load Vector Element Halfword Indexed. EA is aligned down to a
    /// 2-byte boundary before the load.
    Lvehx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-17 s:6.2] lvewx XO=71: word element load to word EA mod 16 of vT.
    /// Load Vector Element Word Indexed. EA is aligned down to a
    /// 4-byte boundary before the load.
    Lvewx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-21 s:6.2] lvx XO=103: 16-byte aligned vector load (EA & ~0xF).
    /// Load Vector Indexed. EA is aligned down to a 16-byte boundary
    /// before the load.
    Lvx {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-29 s:6.2] stvebx XO=135: single-byte element store from byte EA mod 16 of vS.
    /// Store Vector Element Byte Indexed.
    Stvebx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-30 s:6.2] stvehx XO=167: halfword element store from halfword EA mod 16 of vS.
    /// Store Vector Element Halfword Indexed. EA is aligned down to
    /// a 2-byte boundary before the store.
    Stvehx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-31 s:6.2] stvewx XO=199: word element store from word EA mod 16 of vS.
    /// Store Vector Element Word Indexed. EA is aligned down to a
    /// 4-byte boundary before the store.
    Stvewx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-23 s:6.2] lvxl XO=359: lvx with cache-locality "last use" hint.
    /// Load Vector Indexed Last. Same semantics as `Lvx`; the LRU
    /// hint is architecturally a cache directive only.
    Lvxl {
        vt: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-28 s:6.2 AltiVec Instruction Set] stvx X-form (EA aligned down to 16-byte boundary).
    /// Store-vector-indexed. The effective address is aligned down to
    /// a 16-byte boundary before the store.
    Stvx {
        vs: u8,
        ra: u8,
        rb: u8,
    },
    // [AltiVec-PEM p:6-33 s:6.2] stvxl XO=487: stvx with cache-locality "last use" hint.
    /// Store Vector Indexed Last. Same semantics as `Stvx`; the LRU
    /// hint is architecturally a cache directive only.
    Stvxl {
        vs: u8,
        ra: u8,
        rb: u8,
    },

    // -- Floating-point loads/stores --
    // [PPC-Book1 p:104 s:4.6.2 Floating-Point Load Instructions] lfs / lfsx / lfsu / lfsux; lfd at p:105.
    Lfs {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load floating-point single with update. Converts the loaded
    /// single to double in `fpr[frt]`. Requires `ra != 0`.
    // [PPC-Book1 p:104 s:4.6.2 Floating-Point Load Instructions] lfsu D-form.
    Lfsu {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    Lfd {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    /// Load floating-point double with update. Requires `ra != 0`.
    // [PPC-Book1 p:105 s:4.6.2 Floating-Point Load Instructions] lfdu D-form.
    Lfdu {
        frt: u8,
        ra: u8,
        imm: i16,
    },
    // [PPC-Book1 p:107 s:4.6.3 Floating-Point Store Instructions] stfs / stfsu D-form; stfd / stfdu at p:108.
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
    // [PPC-Book1 p:109 s:4.6.3 Floating-Point Store Instructions] stfiwx X-form (low 32 bits stored verbatim).
    /// Store-float-as-integer-word. The low 32 bits of `fpr[frs]` are
    /// written verbatim -- there is no float-to-int conversion.
    Stfiwx {
        frs: u8,
        ra: u8,
        rb: u8,
    },

    // -- X-form floating-point loads/stores (opcode 31) --
    // [PPC-Book1 p:104 s:4.6.2 Floating-Point Load Instructions] lfsx / lfsux / lfdx / lfdux X-form (p:104-105); store X-forms at p:107-108.
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

    // [PPC-Book1 p:111 s:4.6.5 Floating-Point Arithmetic Instructions] primary 63 / 59 dispatch (fadd / fsub / fmul / fdiv / fmadd at p:111-113).
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
    // [PPC-Book1 p:51 s:3.3.8 Fixed-Point Arithmetic Instructions] li/mr/sl(w/d)i/sr(w/d)i/clrl(w/d)i/nop are extended mnemonics for addi/or/rlwinm/rldicl/ori. CmpwZero is a CmpwiZero quickening.
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
    // [PPC-Book2 p:20 s:3.2.1 Cache Management Instructions] dcbz X-form (block-set-to-zero, no caching modeled).
    /// Data cache block set to zero. The 128-byte block containing
    /// `(RA|0)+(RB)` is written with zeros. No cache modelling is
    /// implied; under the deterministic model the visible effect is
    /// a 128-byte zero store at the aligned EA.
    Dcbz {
        ra: u8,
        rb: u8,
    },

    // -- System --
    // [PPC-Book1 p:26 s:2.4.2 System Linkage Instructions] sc SC-form; LEV field selects hypervisor (1) vs kernel (0).
    /// System call. LV2 convention: syscall number in r11. The 7-bit
    /// LEV field selects the privilege level: PS3 usermode always
    /// issues LEV=0 (kernel syscall); LEV=1 would target an LV1
    /// hypercall. Preserved at decode so the executor can route on
    /// it when hypercall dispatch is wired.
    Sc {
        lev: u8,
    },
}

impl PpuInstruction {
    /// Whether this instruction is a fused 2-instruction super-pair
    /// requiring a [`PpuInstruction::Consumed`] placeholder at PC+4.
    /// One-instruction quickenings (`Li`, `Slwi`, `Srwi`, `Clrlwi`)
    /// are NOT super-pairs.
    ///
    /// Exhaustive: every variant must declare its super-pair status
    /// at compile time. Producer side lives in
    /// `crate::shadow::superpair`; the fused variants emitted there
    /// must match the variants returning `true` here. A divergence
    /// causes silent re-execution of the second half of the pair
    /// (see `lib.rs:run_until_yield` `Consumed`-retirement guard).
    pub fn is_super_pair(&self) -> bool {
        match self {
            PpuInstruction::LwzCmpwi { .. }
            | PpuInstruction::LiStw { .. }
            | PpuInstruction::MflrStw { .. }
            | PpuInstruction::LwzMtlr { .. }
            | PpuInstruction::MflrStd { .. }
            | PpuInstruction::LdMtlr { .. }
            | PpuInstruction::StdStd { .. }
            | PpuInstruction::CmpwiBc { .. }
            | PpuInstruction::CmpwBc { .. } => true,
            PpuInstruction::Lwz { .. }
            | PpuInstruction::Lbz { .. }
            | PpuInstruction::Lhz { .. }
            | PpuInstruction::Lha { .. }
            | PpuInstruction::Lhau { .. }
            | PpuInstruction::Lmw { .. }
            | PpuInstruction::Lwzu { .. }
            | PpuInstruction::Lbzu { .. }
            | PpuInstruction::Lhzu { .. }
            | PpuInstruction::Ldu { .. }
            | PpuInstruction::Ld { .. }
            | PpuInstruction::Lwa { .. }
            | PpuInstruction::Stw { .. }
            | PpuInstruction::Stwu { .. }
            | PpuInstruction::Stdu { .. }
            | PpuInstruction::Stb { .. }
            | PpuInstruction::Stbu { .. }
            | PpuInstruction::Stmw { .. }
            | PpuInstruction::Sth { .. }
            | PpuInstruction::Sthu { .. }
            | PpuInstruction::Std { .. }
            | PpuInstruction::Addi { .. }
            | PpuInstruction::Addis { .. }
            | PpuInstruction::Subfic { .. }
            | PpuInstruction::Mulli { .. }
            | PpuInstruction::Addic { .. }
            | PpuInstruction::AddicDot { .. }
            | PpuInstruction::Add { .. }
            | PpuInstruction::Or { .. }
            | PpuInstruction::Subf { .. }
            | PpuInstruction::Subfc { .. }
            | PpuInstruction::Subfe { .. }
            | PpuInstruction::Neg { .. }
            | PpuInstruction::Mullw { .. }
            | PpuInstruction::Mulhwu { .. }
            | PpuInstruction::Mulhw { .. }
            | PpuInstruction::Mulhdu { .. }
            | PpuInstruction::Mulhd { .. }
            | PpuInstruction::Adde { .. }
            | PpuInstruction::Addze { .. }
            | PpuInstruction::Subfze { .. }
            | PpuInstruction::Subfme { .. }
            | PpuInstruction::Addme { .. }
            | PpuInstruction::Mulld { .. }
            | PpuInstruction::Ldarx { .. }
            | PpuInstruction::Stdcx { .. }
            | PpuInstruction::Lwarx { .. }
            | PpuInstruction::Stwcx { .. }
            | PpuInstruction::Xori { .. }
            | PpuInstruction::Xoris { .. }
            | PpuInstruction::Divw { .. }
            | PpuInstruction::Divwu { .. }
            | PpuInstruction::Divd { .. }
            | PpuInstruction::Divdu { .. }
            | PpuInstruction::And { .. }
            | PpuInstruction::Andc { .. }
            | PpuInstruction::Nor { .. }
            | PpuInstruction::Xor { .. }
            | PpuInstruction::Eqv { .. }
            | PpuInstruction::Nand { .. }
            | PpuInstruction::AndiDot { .. }
            | PpuInstruction::AndisDot { .. }
            | PpuInstruction::Slw { .. }
            | PpuInstruction::Srw { .. }
            | PpuInstruction::Srawi { .. }
            | PpuInstruction::Sraw { .. }
            | PpuInstruction::Srad { .. }
            | PpuInstruction::Sradi { .. }
            | PpuInstruction::Sld { .. }
            | PpuInstruction::Srd { .. }
            | PpuInstruction::Cntlzw { .. }
            | PpuInstruction::Cntlzd { .. }
            | PpuInstruction::Popcntb { .. }
            | PpuInstruction::Tw { .. }
            | PpuInstruction::Td { .. }
            | PpuInstruction::Mcrxr { .. }
            | PpuInstruction::Orc { .. }
            | PpuInstruction::Extsh { .. }
            | PpuInstruction::Extsb { .. }
            | PpuInstruction::Extsw { .. }
            | PpuInstruction::Ori { .. }
            | PpuInstruction::Oris { .. }
            | PpuInstruction::Cmpwi { .. }
            | PpuInstruction::Cmplwi { .. }
            | PpuInstruction::Cmpdi { .. }
            | PpuInstruction::Cmpldi { .. }
            | PpuInstruction::Cmpw { .. }
            | PpuInstruction::Cmplw { .. }
            | PpuInstruction::Cmpd { .. }
            | PpuInstruction::Cmpld { .. }
            | PpuInstruction::B { .. }
            | PpuInstruction::Bc { .. }
            | PpuInstruction::Bclr { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::Mcrf { .. }
            | PpuInstruction::Crand { .. }
            | PpuInstruction::Crandc { .. }
            | PpuInstruction::Cror { .. }
            | PpuInstruction::Crorc { .. }
            | PpuInstruction::Crxor { .. }
            | PpuInstruction::Crnand { .. }
            | PpuInstruction::Crnor { .. }
            | PpuInstruction::Creqv { .. }
            | PpuInstruction::Lwzx { .. }
            | PpuInstruction::Lbzx { .. }
            | PpuInstruction::Ldx { .. }
            | PpuInstruction::Lhzx { .. }
            | PpuInstruction::Stwx { .. }
            | PpuInstruction::Stdx { .. }
            | PpuInstruction::Stdux { .. }
            | PpuInstruction::Stbx { .. }
            | PpuInstruction::Lwzux { .. }
            | PpuInstruction::Lbzux { .. }
            | PpuInstruction::Lhzux { .. }
            | PpuInstruction::Ldux { .. }
            | PpuInstruction::Lhax { .. }
            | PpuInstruction::Lhaux { .. }
            | PpuInstruction::Lwax { .. }
            | PpuInstruction::Lwaux { .. }
            | PpuInstruction::Sthx { .. }
            | PpuInstruction::Sthux { .. }
            | PpuInstruction::Stwux { .. }
            | PpuInstruction::Stbux { .. }
            | PpuInstruction::Lswi { .. }
            | PpuInstruction::Lswx { .. }
            | PpuInstruction::Stswi { .. }
            | PpuInstruction::Stswx { .. }
            | PpuInstruction::Ldbrx { .. }
            | PpuInstruction::Lwbrx { .. }
            | PpuInstruction::Lhbrx { .. }
            | PpuInstruction::Sdbrx { .. }
            | PpuInstruction::Stwbrx { .. }
            | PpuInstruction::Sthbrx { .. }
            | PpuInstruction::Mftb { .. }
            | PpuInstruction::Mftbu { .. }
            | PpuInstruction::Mfcr { .. }
            | PpuInstruction::Mtcrf { .. }
            | PpuInstruction::Mfocrf { .. }
            | PpuInstruction::Mtocrf { .. }
            | PpuInstruction::Mflr { .. }
            | PpuInstruction::Mtlr { .. }
            | PpuInstruction::Mfctr { .. }
            | PpuInstruction::Mtctr { .. }
            | PpuInstruction::Mfxer { .. }
            | PpuInstruction::Mtxer { .. }
            | PpuInstruction::Mfvrsave { .. }
            | PpuInstruction::Mtvrsave { .. }
            | PpuInstruction::Rlwinm { .. }
            | PpuInstruction::Rlwimi { .. }
            | PpuInstruction::Rlwnm { .. }
            | PpuInstruction::Rldicl { .. }
            | PpuInstruction::Rldicr { .. }
            | PpuInstruction::Rldic { .. }
            | PpuInstruction::Rldimi { .. }
            | PpuInstruction::Rldcl { .. }
            | PpuInstruction::Rldcr { .. }
            | PpuInstruction::Vx { .. }
            | PpuInstruction::Va { .. }
            | PpuInstruction::Vxor { .. }
            | PpuInstruction::Vsldoi { .. }
            | PpuInstruction::Lvlx { .. }
            | PpuInstruction::Lvrx { .. }
            | PpuInstruction::Lvlxl { .. }
            | PpuInstruction::Lvrxl { .. }
            | PpuInstruction::Stvlx { .. }
            | PpuInstruction::Stvrx { .. }
            | PpuInstruction::Stvlxl { .. }
            | PpuInstruction::Stvrxl { .. }
            | PpuInstruction::Lvsl { .. }
            | PpuInstruction::Lvebx { .. }
            | PpuInstruction::Lvsr { .. }
            | PpuInstruction::Lvehx { .. }
            | PpuInstruction::Lvewx { .. }
            | PpuInstruction::Lvx { .. }
            | PpuInstruction::Stvebx { .. }
            | PpuInstruction::Stvehx { .. }
            | PpuInstruction::Stvewx { .. }
            | PpuInstruction::Lvxl { .. }
            | PpuInstruction::Stvx { .. }
            | PpuInstruction::Stvxl { .. }
            | PpuInstruction::Lfs { .. }
            | PpuInstruction::Lfsu { .. }
            | PpuInstruction::Lfd { .. }
            | PpuInstruction::Lfdu { .. }
            | PpuInstruction::Stfs { .. }
            | PpuInstruction::Stfd { .. }
            | PpuInstruction::Stfsu { .. }
            | PpuInstruction::Stfdu { .. }
            | PpuInstruction::Stfiwx { .. }
            | PpuInstruction::Lfsx { .. }
            | PpuInstruction::Lfsux { .. }
            | PpuInstruction::Lfdx { .. }
            | PpuInstruction::Lfdux { .. }
            | PpuInstruction::Stfsx { .. }
            | PpuInstruction::Stfsux { .. }
            | PpuInstruction::Stfdx { .. }
            | PpuInstruction::Stfdux { .. }
            | PpuInstruction::Fp63 { .. }
            | PpuInstruction::Fp59 { .. }
            | PpuInstruction::Li { .. }
            | PpuInstruction::Mr { .. }
            | PpuInstruction::Slwi { .. }
            | PpuInstruction::Srwi { .. }
            | PpuInstruction::Clrlwi { .. }
            | PpuInstruction::Nop
            | PpuInstruction::CmpwZero { .. }
            | PpuInstruction::Clrldi { .. }
            | PpuInstruction::Sldi { .. }
            | PpuInstruction::Srdi { .. }
            | PpuInstruction::Consumed
            | PpuInstruction::Dcbz { .. }
            | PpuInstruction::Sc { .. } => false,
        }
    }

    /// Whether this variant is a decode-only stub: the decoder
    /// produced it (so the scan counts it as "accepted") but the
    /// executor will fault via `UnimplementedInstruction` for at
    /// least some sub-opcodes. New typed variants are NOT stubs by
    /// default; the predicate names the catch-all family-dispatch
    /// variants explicitly.
    ///
    /// Used by the prescan to keep stubs out of `words_accepted` --
    /// counting a "decodes but doesn't execute" word as accepted
    /// under-reports the gap surface. See
    /// [`crate::prescan::PrescanReport::words_stubbed`].
    pub fn is_stub_variant(&self) -> bool {
        matches!(
            self,
            PpuInstruction::Vx { .. }
                | PpuInstruction::Va { .. }
                | PpuInstruction::Fp59 { .. }
                | PpuInstruction::Fp63 { .. }
        )
    }

    /// Whether this instruction ends a basic block (control-flow
    /// transfer or syscall). Used by the shadow builder to compute
    /// per-slot block lengths.
    ///
    /// Exhaustive: every variant must declare its terminator status
    /// at compile time. A new branch-shaped variant added without
    /// updating this method would silently extend basic-block
    /// boundaries past it, drifting shadow precomputation from the
    /// physical reality of the code.
    pub fn is_block_terminator(&self) -> bool {
        match self {
            PpuInstruction::B { .. }
            | PpuInstruction::Bc { .. }
            | PpuInstruction::Bclr { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::CmpwiBc { .. }
            | PpuInstruction::CmpwBc { .. }
            | PpuInstruction::Sc { .. } => true,
            PpuInstruction::Lwz { .. }
            | PpuInstruction::Lbz { .. }
            | PpuInstruction::Lhz { .. }
            | PpuInstruction::Lha { .. }
            | PpuInstruction::Lhau { .. }
            | PpuInstruction::Lmw { .. }
            | PpuInstruction::Lwzu { .. }
            | PpuInstruction::Lbzu { .. }
            | PpuInstruction::Lhzu { .. }
            | PpuInstruction::Ldu { .. }
            | PpuInstruction::Ld { .. }
            | PpuInstruction::Lwa { .. }
            | PpuInstruction::Stw { .. }
            | PpuInstruction::Stwu { .. }
            | PpuInstruction::Stdu { .. }
            | PpuInstruction::Stb { .. }
            | PpuInstruction::Stbu { .. }
            | PpuInstruction::Stmw { .. }
            | PpuInstruction::Sth { .. }
            | PpuInstruction::Sthu { .. }
            | PpuInstruction::Std { .. }
            | PpuInstruction::Addi { .. }
            | PpuInstruction::Addis { .. }
            | PpuInstruction::Subfic { .. }
            | PpuInstruction::Mulli { .. }
            | PpuInstruction::Addic { .. }
            | PpuInstruction::AddicDot { .. }
            | PpuInstruction::Add { .. }
            | PpuInstruction::Or { .. }
            | PpuInstruction::Subf { .. }
            | PpuInstruction::Subfc { .. }
            | PpuInstruction::Subfe { .. }
            | PpuInstruction::Neg { .. }
            | PpuInstruction::Mullw { .. }
            | PpuInstruction::Mulhwu { .. }
            | PpuInstruction::Mulhw { .. }
            | PpuInstruction::Mulhdu { .. }
            | PpuInstruction::Mulhd { .. }
            | PpuInstruction::Adde { .. }
            | PpuInstruction::Addze { .. }
            | PpuInstruction::Subfze { .. }
            | PpuInstruction::Subfme { .. }
            | PpuInstruction::Addme { .. }
            | PpuInstruction::Mulld { .. }
            | PpuInstruction::Ldarx { .. }
            | PpuInstruction::Stdcx { .. }
            | PpuInstruction::Lwarx { .. }
            | PpuInstruction::Stwcx { .. }
            | PpuInstruction::Xori { .. }
            | PpuInstruction::Xoris { .. }
            | PpuInstruction::Divw { .. }
            | PpuInstruction::Divwu { .. }
            | PpuInstruction::Divd { .. }
            | PpuInstruction::Divdu { .. }
            | PpuInstruction::And { .. }
            | PpuInstruction::Andc { .. }
            | PpuInstruction::Nor { .. }
            | PpuInstruction::Xor { .. }
            | PpuInstruction::Eqv { .. }
            | PpuInstruction::Nand { .. }
            | PpuInstruction::AndiDot { .. }
            | PpuInstruction::AndisDot { .. }
            | PpuInstruction::Slw { .. }
            | PpuInstruction::Srw { .. }
            | PpuInstruction::Srawi { .. }
            | PpuInstruction::Sraw { .. }
            | PpuInstruction::Srad { .. }
            | PpuInstruction::Sradi { .. }
            | PpuInstruction::Sld { .. }
            | PpuInstruction::Srd { .. }
            | PpuInstruction::Cntlzw { .. }
            | PpuInstruction::Cntlzd { .. }
            | PpuInstruction::Popcntb { .. }
            | PpuInstruction::Tw { .. }
            | PpuInstruction::Td { .. }
            | PpuInstruction::Mcrxr { .. }
            | PpuInstruction::Orc { .. }
            | PpuInstruction::Extsh { .. }
            | PpuInstruction::Extsb { .. }
            | PpuInstruction::Extsw { .. }
            | PpuInstruction::Ori { .. }
            | PpuInstruction::Oris { .. }
            | PpuInstruction::Cmpwi { .. }
            | PpuInstruction::Cmplwi { .. }
            | PpuInstruction::Cmpdi { .. }
            | PpuInstruction::Cmpldi { .. }
            | PpuInstruction::Cmpw { .. }
            | PpuInstruction::Cmplw { .. }
            | PpuInstruction::Cmpd { .. }
            | PpuInstruction::Cmpld { .. }
            | PpuInstruction::Mcrf { .. }
            | PpuInstruction::Crand { .. }
            | PpuInstruction::Crandc { .. }
            | PpuInstruction::Cror { .. }
            | PpuInstruction::Crorc { .. }
            | PpuInstruction::Crxor { .. }
            | PpuInstruction::Crnand { .. }
            | PpuInstruction::Crnor { .. }
            | PpuInstruction::Creqv { .. }
            | PpuInstruction::Lwzx { .. }
            | PpuInstruction::Lbzx { .. }
            | PpuInstruction::Ldx { .. }
            | PpuInstruction::Lhzx { .. }
            | PpuInstruction::Stwx { .. }
            | PpuInstruction::Stdx { .. }
            | PpuInstruction::Stdux { .. }
            | PpuInstruction::Stbx { .. }
            | PpuInstruction::Lwzux { .. }
            | PpuInstruction::Lbzux { .. }
            | PpuInstruction::Lhzux { .. }
            | PpuInstruction::Ldux { .. }
            | PpuInstruction::Lhax { .. }
            | PpuInstruction::Lhaux { .. }
            | PpuInstruction::Lwax { .. }
            | PpuInstruction::Lwaux { .. }
            | PpuInstruction::Sthx { .. }
            | PpuInstruction::Sthux { .. }
            | PpuInstruction::Stwux { .. }
            | PpuInstruction::Stbux { .. }
            | PpuInstruction::Lswi { .. }
            | PpuInstruction::Lswx { .. }
            | PpuInstruction::Stswi { .. }
            | PpuInstruction::Stswx { .. }
            | PpuInstruction::Ldbrx { .. }
            | PpuInstruction::Lwbrx { .. }
            | PpuInstruction::Lhbrx { .. }
            | PpuInstruction::Sdbrx { .. }
            | PpuInstruction::Stwbrx { .. }
            | PpuInstruction::Sthbrx { .. }
            | PpuInstruction::Mftb { .. }
            | PpuInstruction::Mftbu { .. }
            | PpuInstruction::Mfcr { .. }
            | PpuInstruction::Mtcrf { .. }
            | PpuInstruction::Mfocrf { .. }
            | PpuInstruction::Mtocrf { .. }
            | PpuInstruction::Mflr { .. }
            | PpuInstruction::Mtlr { .. }
            | PpuInstruction::Mfctr { .. }
            | PpuInstruction::Mtctr { .. }
            | PpuInstruction::Mfxer { .. }
            | PpuInstruction::Mtxer { .. }
            | PpuInstruction::Mfvrsave { .. }
            | PpuInstruction::Mtvrsave { .. }
            | PpuInstruction::Rlwinm { .. }
            | PpuInstruction::Rlwimi { .. }
            | PpuInstruction::Rlwnm { .. }
            | PpuInstruction::Rldicl { .. }
            | PpuInstruction::Rldicr { .. }
            | PpuInstruction::Rldic { .. }
            | PpuInstruction::Rldimi { .. }
            | PpuInstruction::Rldcl { .. }
            | PpuInstruction::Rldcr { .. }
            | PpuInstruction::Vx { .. }
            | PpuInstruction::Va { .. }
            | PpuInstruction::Vxor { .. }
            | PpuInstruction::Vsldoi { .. }
            | PpuInstruction::Lvlx { .. }
            | PpuInstruction::Lvrx { .. }
            | PpuInstruction::Lvlxl { .. }
            | PpuInstruction::Lvrxl { .. }
            | PpuInstruction::Stvlx { .. }
            | PpuInstruction::Stvrx { .. }
            | PpuInstruction::Stvlxl { .. }
            | PpuInstruction::Stvrxl { .. }
            | PpuInstruction::Lvsl { .. }
            | PpuInstruction::Lvebx { .. }
            | PpuInstruction::Lvsr { .. }
            | PpuInstruction::Lvehx { .. }
            | PpuInstruction::Lvewx { .. }
            | PpuInstruction::Lvx { .. }
            | PpuInstruction::Stvebx { .. }
            | PpuInstruction::Stvehx { .. }
            | PpuInstruction::Stvewx { .. }
            | PpuInstruction::Lvxl { .. }
            | PpuInstruction::Stvx { .. }
            | PpuInstruction::Stvxl { .. }
            | PpuInstruction::Lfs { .. }
            | PpuInstruction::Lfsu { .. }
            | PpuInstruction::Lfd { .. }
            | PpuInstruction::Lfdu { .. }
            | PpuInstruction::Stfs { .. }
            | PpuInstruction::Stfd { .. }
            | PpuInstruction::Stfsu { .. }
            | PpuInstruction::Stfdu { .. }
            | PpuInstruction::Stfiwx { .. }
            | PpuInstruction::Lfsx { .. }
            | PpuInstruction::Lfsux { .. }
            | PpuInstruction::Lfdx { .. }
            | PpuInstruction::Lfdux { .. }
            | PpuInstruction::Stfsx { .. }
            | PpuInstruction::Stfsux { .. }
            | PpuInstruction::Stfdx { .. }
            | PpuInstruction::Stfdux { .. }
            | PpuInstruction::Fp63 { .. }
            | PpuInstruction::Fp59 { .. }
            | PpuInstruction::Li { .. }
            | PpuInstruction::Mr { .. }
            | PpuInstruction::Slwi { .. }
            | PpuInstruction::Srwi { .. }
            | PpuInstruction::Clrlwi { .. }
            | PpuInstruction::Nop
            | PpuInstruction::CmpwZero { .. }
            | PpuInstruction::Clrldi { .. }
            | PpuInstruction::Sldi { .. }
            | PpuInstruction::Srdi { .. }
            | PpuInstruction::LwzCmpwi { .. }
            | PpuInstruction::LiStw { .. }
            | PpuInstruction::MflrStw { .. }
            | PpuInstruction::LwzMtlr { .. }
            | PpuInstruction::MflrStd { .. }
            | PpuInstruction::LdMtlr { .. }
            | PpuInstruction::StdStd { .. }
            | PpuInstruction::Consumed
            | PpuInstruction::Dcbz { .. } => false,
        }
    }
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
            let name: &'static str = insn.into();
            assert_eq!(
                name, prefix,
                "IntoStaticStr-derived name mismatch for {debug}",
            );
        }
    }

    #[test]
    fn into_static_str_returns_variant_ident() {
        let insn = PpuInstruction::Add {
            rt: 3,
            ra: 4,
            rb: 5,
            oe: false,
            rc: false,
        };
        let name: &'static str = (&insn).into();
        assert_eq!(name, "Add");
    }
}
