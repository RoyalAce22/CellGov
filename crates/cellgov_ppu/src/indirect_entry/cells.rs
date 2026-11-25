//! The cells of CR, XER and the SPRs that the census scan keeps a lost
//! mark in, and how each word writes and reads them.
//!
//! A word writes only the cells of the bits that it writes. So a compare
//! of a lost GPR into one CR field leaves the other fields, XER and the
//! SPRs without a mark.

use super::constants::{computed_from, gpr_bit, SprMove};
use super::lost::{join_origins, Origins};
use crate::instruction::PpuInstruction;

/// The number of cells.
const CELL_COUNT: usize = 11;

/// A set of cells, with bit `n` set for cell `n`.
///
/// The cells are:
/// - cells 0 to 7: CR fields 0 to 7;
/// - cell 8: XER\[CA\];
/// - cell 9: XER\[OV\] and XER\[SO\];
/// - cell 10: the other bits of XER, and each SPR that the scan does not
///   follow on its own, the MSR and the SLB.
///
/// Cell 10 is one cell for all of its registers, so a move out of any of
/// them takes the mark, also through an SPR alias.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Cells(u16);

impl Cells {
    pub(super) const NONE: Self = Self(0);
    /// Each CR field.
    pub(super) const CR: Self = Self(0xFF);
    pub(super) const CA: Self = Self(1 << 8);
    /// XER\[OV\] and XER\[SO\].
    pub(super) const OV: Self = Self(1 << 9);
    /// Cell 10.
    pub(super) const SPR: Self = Self(1 << 10);
    /// Each cell that holds a bit of XER.
    pub(super) const XER: Self = Self(Self::CA.0 | Self::OV.0 | Self::SPR.0);

    /// CR field `field`, or no cell when `field` is above 7.
    pub(super) fn cr_field(field: u8) -> Self {
        Self(1u16.checked_shl(u32::from(field)).unwrap_or(0) & Self::CR.0)
    }

    /// The CR fields that the field mask `fxm` selects. Bit 0 of FXM, its
    /// most significant bit, selects CR field 0.
    ///
    /// [PPC-Book1 p:83 s:3.3.13] When bit i of FXM is 1, `mtcrf` sets CR field i to the corresponding field of RS 32:63.
    fn cr_fields(fxm: u8) -> Self {
        Self(u16::from(fxm.reverse_bits()))
    }

    /// The cells of `self` and of `other`.
    pub(super) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The cells of `self` that are not in `other`.
    pub(super) const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether `self` and `other` share a cell.
    pub(super) const fn meets(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub(super) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The index of each cell of the set, in ascending order.
    fn indices(self) -> impl Iterator<Item = usize> {
        (0..CELL_COUNT).filter(move |&cell| self.0 & (1 << cell) != 0)
    }

    /// The cells after a write of `write`, when `set` tells whether the
    /// written value holds a mark.
    ///
    /// A write of each bit of a cell gives the cell the new mark alone. A
    /// write of a part of a cell keeps the old mark too.
    pub(super) fn written(self, write: &CellWrite, set: bool) -> Self {
        match (set, write.full) {
            (true, _) => self.union(write.cells),
            (false, true) => self.without(write.cells),
            (false, false) => self,
        }
    }
}

/// One write of a word into the cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CellWrite {
    /// The cells that the word writes.
    pub(super) cells: Cells,
    /// Whether the word writes each bit of each of these cells.
    pub(super) full: bool,
    /// The GPRs that the word computes the written value from, as a mask
    /// with bit `n` set for GPR `n`.
    pub(super) gprs: u32,
    /// The cells that the word computes the written value from.
    pub(super) from: Cells,
}

impl CellWrite {
    /// A write of each bit of `cells` from `gprs` and `from`.
    fn full(cells: Cells, gprs: u32, from: Cells) -> Self {
        Self {
            cells,
            full: true,
            gprs,
            from,
        }
    }

    /// A write of a part of `cells` from `gprs` and `from`.
    fn part(cells: Cells, gprs: u32, from: Cells) -> Self {
        Self {
            cells,
            full: false,
            gprs,
            from,
        }
    }
}

/// The writes of `decoded` into the cells, each one computed from the
/// GPRs and cells before the word.
///
/// `moved` is how the word moves a value between the GPRs and CR or an
/// SPR that the scan does not follow, as the scan's own move table gives
/// it. An unused entry writes no cell.
///
/// These words write cells from a GPR:
/// - `mtcrf` writes each CR field that FXM selects;
/// - `mtocrf` writes the CR field that FXM selects when FXM has one bit
///   set, and else a part of each field that FXM selects;
/// - `mtxer` writes XER\[CA\], XER\[OV\] and XER\[SO\], and a part of
///   cell 10;
/// - each other move into a register writes a part of cell 10;
/// - each word of [`flag_writes`].
///
/// For an FXM without exactly one bit set, the PPE writes only the
/// leftmost field that FXM selects, and no field for an FXM of 0, on each
/// such FXM that the reference console ran. So an `mtocrf` leaves each
/// field that FXM does not select unchanged.
///
/// [PPC-Book1 p:124 s:5.1.1] When FXM has exactly one bit set, `mtocrf` places RS into CR field n; otherwise the contents of CR are undefined.
/// [PPC-Book1 p:32 s:3.2.2] An `mtspr` to XER sets SO and OV from the values that it supplies.
pub(super) fn cell_writes(decoded: &PpuInstruction, moved: Option<SprMove>) -> [CellWrite; 3] {
    let none = CellWrite::default();
    let Some(SprMove::Into(gprs)) = moved else {
        return match moved {
            Some(SprMove::OutOf(_)) => [none; 3],
            _ => flag_writes(decoded),
        };
    };
    let write = match *decoded {
        PpuInstruction::Mtcrf { crm, .. } => {
            CellWrite::full(Cells::cr_fields(crm), gprs, Cells::NONE)
        }
        PpuInstruction::Mtocrf { crm, .. } if crm.count_ones() == 1 => {
            CellWrite::full(Cells::cr_fields(crm), gprs, Cells::NONE)
        }
        PpuInstruction::Mtocrf { crm, .. } => {
            CellWrite::part(Cells::cr_fields(crm), gprs, Cells::NONE)
        }
        PpuInstruction::Mtxer { .. } => {
            return [
                CellWrite::full(Cells::CA.union(Cells::OV), gprs, Cells::NONE),
                CellWrite::part(Cells::SPR, gprs, Cells::NONE),
                none,
            ];
        }
        _ => CellWrite::part(Cells::SPR, gprs, Cells::NONE),
    };
    [write, none, none]
}

/// The cells that `decoded` moves into a GPR, for a word that the scan's
/// move table gives as a move out of CR or an SPR.
///
/// `mfocrf` takes each CR field. The PPE places the whole CR into RT for
/// each FXM that the reference console ran, and the documents leave the
/// other fields undefined.
///
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT 32:63.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` of XER places XER into RT.
pub(super) fn read_cells(decoded: &PpuInstruction) -> Cells {
    match decoded {
        PpuInstruction::Mfcr { .. } | PpuInstruction::Mfocrf { .. } => Cells::CR,
        PpuInstruction::Mfxer { .. } => Cells::XER,
        _ => Cells::SPR,
    }
}

/// The cells that the word `decoded` of [`computed_from`] computes its
/// GPR result from: XER\[CA\] for a word that adds CA into its result.
///
/// [PPC-Book1 p:54 s:3.3.8] `adde`, `subfe`, `addme` and `subfme` add CA into the sum that they place into RT.
/// [PPC-Book1 p:55 s:3.3.8] `addze` and `subfze` add CA into the sum that they place into RT.
pub(super) fn computed_cells(decoded: &PpuInstruction) -> Cells {
    use PpuInstruction as I;
    match decoded {
        I::Adde { .. }
        | I::Subfe { .. }
        | I::Addme { .. }
        | I::Subfme { .. }
        | I::Addze { .. }
        | I::Subfze { .. } => Cells::CA,
        _ => Cells::NONE,
    }
}

/// The writes of `decoded` into the cells of CR and XER, for a word that
/// moves no value between a GPR and CR or an SPR.
///
/// These words write cells:
/// - a compare writes CR field BF from RA, RB and XER\[SO\];
/// - `stwcx.` and `stdcx.` write CR0 from the EA that RA and RB give, and
///   from XER\[SO\];
/// - `mcrf` writes CR field BF from CR field BFA;
/// - a CR logical word writes a part of the field of BT, from the fields
///   of BA and BB;
/// - `mcrxr` writes CR field BF from XER\[CA\], XER\[OV\] and XER\[SO\];
/// - a word of [`computed_from`] writes XER\[CA\] when it sets CA, a part
///   of XER\[OV\] and XER\[SO\] with OE = 1, and CR0 with Rc = 1. Each of
///   these comes from the sources of the result, and CR0 also from
///   XER\[SO\].
///
/// The PPE does not implement `mcrxr`. The bound keeps the path after it,
/// because a handler can emulate it.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book2 p:25 s:3.3] `stwcx.` and `stdcx.` set CR0 to 0b00 || n || XER\[SO\], with n from whether the reservation address is the real address of EA = (RA|0) + (RB).
/// [PPC-Book1 p:30 s:2.4.4] `mcrf` copies CR field BFA into CR field BF.
/// [PPC-Book1 p:28 s:2.4.3] `crand`, `cror`, `crxor` and `crnand` place a function of the CR bits BA and BB into the CR bit BT.
/// [PPC-Book1 p:29 s:2.4.3] `crnor`, `creqv`, `crandc` and `crorc` place a function of the CR bits BA and BB into the CR bit BT.
/// [PPC-Book1 p:135 s:6.1] `mcrxr` copies XER 32:35 into CR field BF.
pub(super) fn flag_writes(decoded: &PpuInstruction) -> [CellWrite; 3] {
    use PpuInstruction as I;
    let none = CellWrite::default();
    let base = |ra: u8| if ra == 0 { 0 } else { gpr_bit(ra) };
    let field = Cells::cr_field;
    let write = match *decoded {
        I::Cmpwi { bf, ra, .. }
        | I::Cmplwi { bf, ra, .. }
        | I::Cmpdi { bf, ra, .. }
        | I::Cmpldi { bf, ra, .. }
        | I::CmpwZero { bf, ra }
        | I::CmpdZero { bf, ra } => CellWrite::full(field(bf), gpr_bit(ra), Cells::OV),
        I::Cmpw { bf, ra, rb }
        | I::Cmplw { bf, ra, rb }
        | I::Cmpd { bf, ra, rb }
        | I::Cmpld { bf, ra, rb } => {
            CellWrite::full(field(bf), gpr_bit(ra) | gpr_bit(rb), Cells::OV)
        }
        I::Stwcx { ra, rb, .. } | I::Stdcx { ra, rb, .. } => {
            CellWrite::full(field(0), base(ra) | gpr_bit(rb), Cells::OV)
        }
        I::Mcrf { crfd, crfs } => CellWrite::full(field(crfd), 0, field(crfs)),
        I::Crand { bt, ba, bb }
        | I::Crandc { bt, ba, bb }
        | I::Cror { bt, ba, bb }
        | I::Crorc { bt, ba, bb }
        | I::Crxor { bt, ba, bb }
        | I::Crnand { bt, ba, bb }
        | I::Crnor { bt, ba, bb }
        | I::Creqv { bt, ba, bb } => {
            CellWrite::part(field(bt / 4), 0, field(ba / 4).union(field(bb / 4)))
        }
        I::Mcrxr { bf } => CellWrite::full(field(bf), 0, Cells::CA.union(Cells::OV)),
        _ => return computed_flag_writes(decoded),
    };
    [write, none, none]
}

/// The writes of the word `decoded` of [`computed_from`] into XER\[CA\],
/// XER\[OV\] and XER\[SO\], and CR0.
///
/// XER\[SO\] stays set once a word sets it, so a write with OE = 1 writes
/// a part of its cell.
///
/// [PPC-Book1 p:51 s:3.3.8] `addic`, `addic.`, `subfic`, `addc`, `subfc`, `adde`, `subfe`, `addme`, `subfme`, `addze` and `subfze` always set CA from the carry of the result.
/// [PPC-Book1 p:79 s:3.3.12.2] `sradi` and `srawi` set CA from RS and the bits that they shift out.
/// [PPC-Book1 p:80 s:3.3.12.2] `srad` and `sraw` set CA from RS and the bits that they shift out.
/// [PPC-Book1 p:32 s:3.2.2] An XO-form add, subtract from, negate, multiply low or divide with OE = 1 sets OV, and an instruction that sets OV sets SO.
/// [PPC-Book1 p:18 s:2.3.1] A fixed-point instruction with Rc = 1, and `addic.`, `andi.` and `andis.`, set CR0 from a compare of the result with 0 and from XER\[SO\].
fn computed_flag_writes(decoded: &PpuInstruction) -> [CellWrite; 3] {
    let none = CellWrite::default();
    let Some((_, gprs)) = computed_from(decoded) else {
        return [none; 3];
    };
    let FlagBits { ca, oe, rc } = FlagBits::of(decoded);
    let from = computed_cells(decoded);
    [
        if ca {
            CellWrite::full(Cells::CA, gprs, from)
        } else {
            none
        },
        if oe {
            CellWrite::part(Cells::OV, gprs, from)
        } else {
            none
        },
        if rc {
            CellWrite::full(Cells::cr_field(0), gprs, from.union(Cells::OV))
        } else {
            none
        },
    ]
}

/// Which flags a word of [`computed_from`] sets.
#[derive(Debug, Clone, Copy, Default)]
struct FlagBits {
    /// The word sets XER\[CA\].
    ca: bool,
    /// The word has OE = 1, so it sets XER\[OV\] and can set XER\[SO\].
    oe: bool,
    /// The word sets CR0.
    rc: bool,
}

impl FlagBits {
    fn of(decoded: &PpuInstruction) -> Self {
        use PpuInstruction as I;
        let carry = |oe, rc| Self { ca: true, oe, rc };
        let record = |rc| Self {
            rc,
            ..Self::default()
        };
        match *decoded {
            I::Addic { .. } | I::Subfic { .. } => carry(false, false),
            I::AddicDot { .. } => carry(false, true),
            I::Addc { oe, rc, .. }
            | I::Subfc { oe, rc, .. }
            | I::Adde { oe, rc, .. }
            | I::Subfe { oe, rc, .. }
            | I::Addme { oe, rc, .. }
            | I::Subfme { oe, rc, .. }
            | I::Addze { oe, rc, .. }
            | I::Subfze { oe, rc, .. } => carry(oe, rc),
            I::Sraw { rc, .. } | I::Srawi { rc, .. } | I::Srad { rc, .. } | I::Sradi { rc, .. } => {
                carry(false, rc)
            }
            I::AndiDot { .. } | I::AndisDot { .. } => record(true),
            I::Add { oe, rc, .. }
            | I::Subf { oe, rc, .. }
            | I::Neg { oe, rc, .. }
            | I::Mullw { oe, rc, .. }
            | I::Mulld { oe, rc, .. }
            | I::Divw { oe, rc, .. }
            | I::Divwu { oe, rc, .. }
            | I::Divd { oe, rc, .. }
            | I::Divdu { oe, rc, .. } => Self { ca: false, oe, rc },
            I::Or { rc, .. }
            | I::Orc { rc, .. }
            | I::And { rc, .. }
            | I::Andc { rc, .. }
            | I::Nor { rc, .. }
            | I::Xor { rc, .. }
            | I::Eqv { rc, .. }
            | I::Nand { rc, .. }
            | I::Slw { rc, .. }
            | I::Srw { rc, .. }
            | I::Sld { rc, .. }
            | I::Srd { rc, .. }
            | I::Mulhwu { rc, .. }
            | I::Mulhw { rc, .. }
            | I::Mulhdu { rc, .. }
            | I::Mulhd { rc, .. }
            | I::Extsh { rc, .. }
            | I::Extsb { rc, .. }
            | I::Extsw { rc, .. }
            | I::Cntlzw { rc, .. }
            | I::Cntlzd { rc, .. }
            | I::Rlwinm { rc, .. }
            | I::Rlwimi { rc, .. }
            | I::Rlwnm { rc, .. }
            | I::Rldicl { rc, .. }
            | I::Rldicr { rc, .. }
            | I::Rldic { rc, .. }
            | I::Rldimi { rc, .. }
            | I::Rldcl { rc, .. }
            | I::Rldcr { rc, .. } => record(rc),
            _ => Self::default(),
        }
    }
}

/// The lost mark of each cell, for the scan of the code constants.
///
/// Each mark is stepped, because a move out can take only a part of the
/// value. The scan keeps no array while no cell holds a mark.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CellMarks(Option<Box<[Option<Origins>; CELL_COUNT]>>);

impl CellMarks {
    /// The origins of each mark that a cell of `cells` holds, or `None`
    /// when no cell of `cells` holds one.
    pub(super) fn read(&self, cells: Cells) -> Option<Origins> {
        let marks = self.0.as_ref()?;
        let mut origins = None;
        for cell in cells.indices() {
            join_origins(&mut origins, marks[cell].as_ref());
        }
        origins
    }

    /// Writes `origins`, the mark of the written value, into the cells of
    /// `write`.
    ///
    /// A write of each bit of a cell gives the cell `origins` alone. A
    /// write of a part of a cell joins `origins` into its old mark.
    pub(super) fn write(&mut self, write: &CellWrite, origins: Option<&Origins>) {
        if origins.is_none() && (!write.full || self.0.is_none()) {
            return;
        }
        let marks = self.0.get_or_insert_with(Box::default);
        for cell in write.cells.indices() {
            if write.full {
                marks[cell] = origins.cloned();
            } else {
                join_origins(&mut marks[cell], origins);
            }
        }
        if marks.iter().all(Option::is_none) {
            self.0 = None;
        }
    }

    /// Joins the marks of `other` into `self`, cell by cell, and returns
    /// whether `self` changed.
    pub(super) fn join(&mut self, other: &Self) -> bool {
        let Some(path) = other.0.as_ref() else {
            return false;
        };
        let marks = self.0.get_or_insert_with(Box::default);
        let mut changed = false;
        for (own, path) in marks.iter_mut().zip(path.iter()) {
            changed |= join_origins(own, path.as_ref());
        }
        changed
    }
}
