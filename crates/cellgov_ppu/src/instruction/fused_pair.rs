//! [`FusedPair`]: the two-instruction super-instruction variants.

#![allow(missing_docs)]

/// A fused pair: two adjacent decoded words the shadow runs as one slot.
///
/// The second word's slot holds
/// [`PpuInstruction::Consumed`](super::PpuInstruction::Consumed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::EnumDiscriminants)]
#[strum_discriminants(
    name(FusedPairKind),
    derive(PartialOrd, Ord, strum::VariantArray, strum::IntoStaticStr)
)]
pub enum FusedPair {
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
    /// Two adjacent `ld` loads from one base with `off2 == off1 + 8`.
    ///
    /// The shadow fuses the pair only when `rt1` is not a nonzero `ra`, so the
    /// first load keeps the base and EA2 is EA1 + 8.
    LdLd {
        rt1: u8,
        rt2: u8,
        ra: u8,
        offset1: i16,
    },
    /// `lbzu rT, D(rA)` followed by `cmpwi crF, rC, 0`, where `rC` is `cmp_ra`.
    ///
    /// The shadow fuses only a valid `lbzu`, with `ra` neither 0 nor `rt`.
    LbzuCmpwi {
        rt: u8,
        ra: u8,
        offset: i16,
        bf: u8,
        cmp_ra: u8,
    },
    /// `ldu rT1, off1(rA1)` followed by `ldu rT2, off2(rA2)`.
    ///
    /// The shadow fuses only valid halves whose second base is not `rt1`.
    /// The second base thus holds EA1 when `ra2 == ra1`, and its own value
    /// otherwise.
    LduLdu {
        rt1: u8,
        ra1: u8,
        off1: i16,
        rt2: u8,
        ra2: u8,
        off2: i16,
    },
    /// `cmpd crF, rA, rB` followed by `nor rA2, rS, rB2` with Rc = 0.
    CmpdNor {
        bf: u8,
        ra: u8,
        rb: u8,
        nor_ra: u8,
        nor_rs: u8,
        nor_rb: u8,
    },
    /// `extsw rA, rS` with Rc = 0, followed by a branch-always `bclr`
    /// (the quickened `blr`).
    ExtswBlr {
        ra: u8,
        rs: u8,
        bo: u8,
        bi: u8,
    },
    /// `or rA, rS, rB` (or its quickened `mr`) followed by
    /// `add rT, rA2, rB2`. Neither half sets Rc, and the `add` has OE = 0.
    OrAdd {
        ra: u8,
        rs: u8,
        rb: u8,
        rt: u8,
        add_ra: u8,
        add_rb: u8,
    },
    /// `lis rT, hi` followed by `ori rT, rT, lo`: `hi` is the `lis`
    /// immediate, and `lo` is the `ori` immediate.
    LisOri {
        rt: u8,
        hi: i16,
        lo: u16,
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
    /// `cmpdi crF, rA, imm` (or its quickened zero form) followed by a
    /// non-linking relative `bc`.
    CmpdiBc {
        bf: u8,
        ra: u8,
        imm: i16,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    /// `cmpd crF, rA, rB` followed by a non-linking relative `bc`.
    CmpdBc {
        bf: u8,
        ra: u8,
        rb: u8,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    /// `cmplw crF, rA, rB` followed by a non-linking relative `bc`.
    CmplwBc {
        bf: u8,
        ra: u8,
        rb: u8,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    /// `or rA, rS, rB` with Rc = 0 (or its quickened `mr`) followed by
    /// `rlwinm rA2, rS2, sh, mb, me` (or a quickened form of it), with `rc`
    /// the rlwinm's Rc bit.
    OrRlwinm {
        ra: u8,
        rs: u8,
        rb: u8,
        rlwinm_ra: u8,
        rlwinm_rs: u8,
        sh: u8,
        mb: u8,
        me: u8,
        rc: bool,
    },
    /// `lwz rT1, off1(rA1)` followed by `lwz rT2, off2(rA2)`.
    ///
    /// The shadow fuses the pair only when `ra2` is 0 or not `rt1`, so EA2
    /// does not depend on the first load's value.
    LwzLwz {
        rt1: u8,
        ra1: u8,
        off1: i16,
        rt2: u8,
        ra2: u8,
        off2: i16,
    },
    /// `ld rT, offset(r1)` followed by `rlwinm rA, rS, sh, mb, me` (or a
    /// quickened form of it), with `rc` the rlwinm's Rc bit.
    LdRlwinm {
        rt: u8,
        offset: i16,
        ra: u8,
        rs: u8,
        sh: u8,
        mb: u8,
        me: u8,
        rc: bool,
    },
    /// `addi rT, rA, imm` (or its quickened `li` when `ra` is 0) followed
    /// by a non-linking relative `bc`.
    AddiBc {
        rt: u8,
        ra: u8,
        imm: i16,
        bo: u8,
        bi: u8,
        target_offset: i16,
    },
    /// `std rS, store_offset(r1)` followed by `lwz rT, offset(rA)`.
    ///
    /// The `lwz` reads memory after the `std` wrote it, so where the two
    /// accesses overlap, the `lwz` gets the stored bytes.
    StdLwz {
        rs: u8,
        store_offset: i16,
        rt: u8,
        ra: u8,
        offset: i16,
    },
    /// `ld rT1, off1(rA1)` followed by `ld rT2, off2(rA2)`, where the pair
    /// is not a [`FusedPair::LdLd`].
    ///
    /// The shadow fuses the pair only when `ra2` is 0 or not `rt1`, so EA2
    /// does not depend on the first load's value.
    LdLdAny {
        rt1: u8,
        ra1: u8,
        off1: i16,
        rt2: u8,
        ra2: u8,
        off2: i16,
    },
    /// `or rA, rS, rB` (or its quickened `mr`), with `rc` its Rc bit,
    /// followed by a relative `b` with LK = 0. `offset` is the branch
    /// displacement from the second slot.
    OrB {
        ra: u8,
        rs: u8,
        rb: u8,
        rc: bool,
        offset: super::BranchOffset,
    },
    /// `lbz rT1, off1(rA1)` followed by `lbz rT2, off2(rA2)`.
    ///
    /// The shadow fuses the pair only when `ra2` is 0 or not `rt1`, so EA2
    /// does not depend on the first load's value.
    LbzLbz {
        rt1: u8,
        ra1: u8,
        off1: i16,
        rt2: u8,
        ra2: u8,
        off2: i16,
    },
    /// `mtlr rS` followed by `addi rT, rA, imm` (or its quickened `li`
    /// when `ra` is 0).
    MtlrAddi {
        rs: u8,
        rt: u8,
        ra: u8,
        imm: i16,
    },
    /// `rldicl rA1, rS1, sh1, mb1` followed by `rldicl rA2, rS2, sh2, mb2`
    /// (or a quickened `clrldi` or `srdi` in either slot). Bit 0 of `rc` is
    /// the first half's Rc bit, and bit 1 is the second half's.
    RldiclRldicl {
        ra1: u8,
        rs1: u8,
        sh1: u8,
        mb1: u8,
        ra2: u8,
        rs2: u8,
        sh2: u8,
        mb2: u8,
        rc: u8,
    },
}
