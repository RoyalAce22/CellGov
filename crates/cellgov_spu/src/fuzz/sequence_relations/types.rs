//! The row types of the sequence-relation catalog.

use crate::instruction::SpuInstructionKind;
use crate::state::SpuState;

use super::super::classify::form_for_kind;
use super::super::metamorphic::opcode_word;
use super::super::types::SpuEncodingForm;

/// Local-store address a row program starts at. It sits mid-store, so an
/// access that wraps at the local-store limit lands away from the program.
pub const SEQUENCE_PROGRAM_BASE: u32 = 0x2_0000;

/// Local-store address of the taken landing: the terminator a branch in a
/// row targets. A row program and its fall-through terminator end below it.
pub const SEQUENCE_TAKEN_LANDING: u32 = SEQUENCE_PROGRAM_BASE + 0x100;

/// One sequence relation of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuSequenceRelationId {
    /// `ceq c,a,b; ceqi rt,c,0` against the fused `sext(a != b)`, which
    /// also writes `c`.
    CeqNotEqualFused,
    /// `ceq c,a,b; ceqi rt,c,0` against `ceq c,a,b; nor rt,c,c`.
    CeqNotEqualNor,
    /// `ceq c,a,b; ceqi rt,c,0` against a fused `sext(a != b)` that writes
    /// only `rt`: valid only while `c` is dead.
    CeqNotEqualResultOnly,
    /// `ceqh c,a,b; ceqhi rt,c,0` against the fused halfword `sext(a != b)`.
    CeqhNotEqualFused,
    /// The five-instruction 32-bit lane multiply against a fused `a*b`.
    Mpy32,
    /// [`Self::Mpy32`] with the operands of the final `a` swapped.
    Mpy32Swapped,
    /// `ceq` feeding `selb`, against a fused word lane select.
    SelectCeq,
    /// `ceqh` feeding `selb`.
    SelectCeqh,
    /// `ceqb` feeding `selb`.
    SelectCeqb,
    /// `ceqi` feeding `selb`.
    SelectCeqi,
    /// `ceqhi` feeding `selb`.
    SelectCeqhi,
    /// `ceqbi` feeding `selb`.
    SelectCeqbi,
    /// `cgt` feeding `selb`.
    SelectCgt,
    /// `cgth` feeding `selb`.
    SelectCgth,
    /// `cgtb` feeding `selb`.
    SelectCgtb,
    /// `cgti` feeding `selb`.
    SelectCgti,
    /// `cgthi` feeding `selb`.
    SelectCgthi,
    /// `cgtbi` feeding `selb`.
    SelectCgtbi,
    /// `clgt` feeding `selb`.
    SelectClgt,
    /// `clgth` feeding `selb`.
    SelectClgth,
    /// `clgtb` feeding `selb`.
    SelectClgtb,
    /// `clgti` feeding `selb`.
    SelectClgti,
    /// `clgthi` feeding `selb`.
    SelectClgthi,
    /// `clgtbi` feeding `selb`.
    SelectClgtbi,
    /// `fceq` feeding `selb`.
    SelectFceq,
    /// `fcgt` feeding `selb`.
    SelectFcgt,
    /// `fcmeq` feeding `selb`.
    SelectFcmeq,
    /// `fcmgt` feeding `selb`.
    SelectFcmgt,
    /// `ceq` feeding `fsm`, against a fused splat of the preferred compare.
    SplatCeq,
    /// `cgt` feeding `fsm`.
    SplatCgt,
    /// `clgt` feeding `fsm`.
    SplatClgt,
    /// `cbd` feeding `shufb`, against a fused byte insert.
    InsertCbd,
    /// `chd` feeding `shufb`.
    InsertChd,
    /// `cwd` feeding `shufb`.
    InsertCwd,
    /// `cdd` feeding `shufb`.
    InsertCdd,
    /// `cbx` feeding `shufb`.
    InsertCbx,
    /// `chx` feeding `shufb`.
    InsertChx,
    /// `cwx` feeding `shufb`.
    InsertCwx,
    /// `cdx` feeding `shufb`.
    InsertCdx,
    /// `sfi n,x,0; rotm rt,a,n` against a fused right shift by `x`.
    NegatedCountRotm,
    /// `sfi n,x,0; rotma rt,a,n`.
    NegatedCountRotma,
    /// `sfhi n,x,0; rothm rt,a,n`.
    NegatedCountRothm,
    /// `sfhi n,x,0; rotmah rt,a,n`.
    NegatedCountRotmah,
    /// `sfi n,x,0; rotqmbi rt,a,n`.
    NegatedCountRotqmbi,
    /// `sfi n,x,0; rotqmby rt,a,n`.
    NegatedCountRotqmby,
    /// `rotqbybi v,x,s; rotqbi rt,v,s` against a fused quadword rotate.
    FunnelShift,
    /// `orx o,v; brz o` against a fused test of `v`.
    BranchOrxBrz,
    /// `orx o,v; brnz o`.
    BranchOrxBrnz,
    /// `orx o,v; biz o,t`.
    BranchOrxBiz,
    /// `orx o,v; binz o,t`.
    BranchOrxBinz,
    /// `cntb c,a; sumb rt,c,c` against a fused per-word popcount.
    Popcount,
    /// `ai x,y,C; lqd rt,i(x)` against `ai x,y,C; lqd rt,(i+C)(y)`.
    SplitAddressLoad,
    /// `ai x,y,C; stqd r,i(x)` against `ai x,y,C; stqd r,(i+C)(y)`.
    SplitAddressStore,
    /// `ori m,x,0; a rt,m,y` against `ai m,x,0; a rt,m,y`.
    MoveOriAi,
    /// `ori m,x,0; a rt,m,y` against `andi m,x,-1; a rt,m,y`.
    MoveOriAndi,
    /// `ori m,x,0; a rt,m,y` against `shlqbyi m,x,0; a rt,m,y`.
    MoveOriShlqbyi,
    /// `frest t,x; fi rt,x,t` against the truncated reciprocal, measured.
    EstimateReciprocal,
    /// `frsqest t,x; fi rt,x,t` against the truncated reciprocal square root.
    EstimateRsqrt,
    /// The documented Newton reciprocal against the truncated reciprocal,
    /// within 1 ulp.
    NewtonReciprocal,
    /// [`Self::NewtonReciprocal`] with `one` one ulp above 1.0, measured.
    NewtonReciprocalOnePlus,
    /// The documented reciprocal square root Newton step, within 1 ulp.
    RsqrtNewton,
    /// The reciprocal square root chain that yields `sqrt(x)`, against the
    /// host's square root, measured.
    SquareRoot,
    /// The reciprocal and correction chain that yields `a / b`, against the
    /// host's division, measured.
    Division,
    /// `fcgt c,a,b; selb rt,b,a,c` against the host's maximum.
    FloatMax,
    /// `fcgt c,a,b; selb rt,a,b,c` against the host's minimum.
    FloatMin,
    /// `fcmgt c,a,b; selb rt,b,a,c` against the host's larger magnitude.
    MagnitudeMax,
    /// `fcmgt c,a,b; selb rt,a,b,c` against the host's smaller magnitude.
    MagnitudeMin,
    /// `fceq c,a,b; selb rt,b,a,c` against the host's equal pick.
    EqualPick,
    /// `fcgt c,a,b; selb rt,b,a,c` against the SPU lane select.
    FloatMaxSelect,
}

/// How exactly a relation's partner matches its sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuFloatClass {
    /// Bit-exact on every start state.
    BitExact,
    /// Bit-exact on every start state the precondition admits.
    BitExactUnderPrecondition,
    /// Inexact: the registers the row names approximate may differ in
    /// low-order bits.
    ///
    /// [Schkufza2014 p:56 s:3] The error in a register is its distance in
    /// ULPs from the target's value; a bound on it is the claim.
    Inexact {
        /// The largest ULP distance the claim admits in each lane; `None`
        /// when the row only measures it.
        ulp: Option<u32>,
    },
}

/// One instruction of a relation row, with symbolic registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuSymbolicWord {
    /// The instruction kind.
    pub kind: SpuInstructionKind,
    /// Symbolic RT.
    pub rt: u8,
    /// Symbolic RA.
    pub ra: u8,
    /// Symbolic RB; unused by an immediate form.
    pub rb: u8,
    /// Symbolic RC of a four-register form.
    pub rc: u8,
    /// The immediate field, or a relative branch's signed word offset.
    pub imm: u32,
}

impl SpuSymbolicWord {
    /// The word with each symbolic register replaced by `assignment[index]`;
    /// `None` for an encoding form the rows do not use, or an index past
    /// the assignment.
    pub fn encode(&self, assignment: &[u8]) -> Option<u32> {
        use SpuInstructionKind as K;
        let register = |index: u8| assignment.get(usize::from(index)).map(|r| u32::from(*r));
        let opcode = opcode_word(self.kind)?;
        let rt = register(self.rt)?;
        Some(match form_for_kind(self.kind) {
            SpuEncodingForm::Rrr => {
                opcode | register(self.rb)? << 14 | register(self.ra)? << 7 | rt
            }
            // [SPU-ISA p:28 s:2.3] RRR: RT in bits 4:10, then RB, RA and RC.
            SpuEncodingForm::Rrrr => {
                opcode
                    | rt << 21
                    | register(self.rb)? << 14
                    | register(self.ra)? << 7
                    | register(self.rc)?
            }
            SpuEncodingForm::Ri10 => {
                opcode | (self.imm & 0x3FF) << 14 | register(self.ra)? << 7 | rt
            }
            SpuEncodingForm::Ri7 => opcode | (self.imm & 0x7F) << 14 | register(self.ra)? << 7 | rt,
            SpuEncodingForm::Branch => match self.kind {
                // [SPU-ISA p:182 s:7 Brnz] RI16: I16 in bits 9:24, RT in 25:31.
                K::Brz | K::Brnz | K::Brhz | K::Brhnz => opcode | (self.imm & 0xFFFF) << 7 | rt,
                // [SPU-ISA p:186 s:7 Biz] RR: RA in bits 18:24, RT in 25:31.
                K::Biz | K::Binz | K::Bihz | K::Bihnz => opcode | register(self.ra)? << 7 | rt,
                _ => return None,
            },
            _ => return None,
        })
    }
}

/// Where a fused partner leaves control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuFusedFlow {
    /// Past the fused sequence.
    FallThrough,
    /// At the taken landing, [`SEQUENCE_TAKEN_LANDING`].
    Taken,
}

/// A fused partner: an operation on the start state, and the symbolic
/// registers it writes.
#[derive(Debug, Clone, Copy)]
pub struct SpuFusedReference {
    /// The symbolic registers the operation writes.
    pub writes: &'static [u8],
    /// Applies the operation to `state` under `assignment`, and says where
    /// control goes.
    pub apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
}

/// The partner a relation compares its sequence against.
#[derive(Debug, Clone, Copy)]
pub enum SpuSequencePartner {
    /// Another guest sequence.
    Guest(&'static [SpuSymbolicWord]),
    /// A fused reference.
    Fused(SpuFusedReference),
}

/// A start-state test under `assignment`: true when the relation claims
/// the partner matches.
pub type SpuSequencePrecondition = fn(&SpuState, &[u8]) -> bool;

/// A value the instantiation places in one symbolic register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuSequencePin {
    /// The preferred word holds [`SEQUENCE_TAKEN_LANDING`], an indirect
    /// branch's target.
    TakenLanding,
    /// Every word holds this value: a constant a row reads, such as 1.0.
    Word(u32),
}

/// One row of the sequence-relation catalog.
#[derive(Debug, Clone, Copy)]
pub struct SpuSequenceRelation {
    /// The row's identity.
    pub id: SpuSequenceRelationId,
    /// Sequence A.
    pub sequence: &'static [SpuSymbolicWord],
    /// Partner B.
    pub partner: SpuSequencePartner,
    /// The start states the claim covers; `None` covers every state.
    pub precondition: Option<SpuSequencePrecondition>,
    /// How exactly B matches A.
    pub float_class: SpuFloatClass,
    /// The symbolic registers B may leave with another value: the claim
    /// holds only while they are dead at exit. Empty means the complete
    /// observation.
    ///
    /// [Mullen2016 p:449 s:1] A rewrite names the registers that must be
    /// dead for it to apply.
    pub dead: &'static [u8],
    /// Registers the instantiation sets to a fixed value.
    pub pins: &'static [(u8, SpuSequencePin)],
    /// True when the row reads or writes local store, so the start state
    /// needs local-store contents.
    pub local_store: bool,
    /// The symbolic registers an inexact row compares lane by lane within
    /// its ULP bound instead of bit for bit.
    pub approximate: &'static [u8],
}

impl SpuSequenceRelation {
    /// The number of symbolic registers the row names.
    pub fn register_count(&self) -> usize {
        let partner: &[SpuSymbolicWord] = match self.partner {
            SpuSequencePartner::Guest(words) => words,
            SpuSequencePartner::Fused(_) => &[],
        };
        let fused_writes = match self.partner {
            SpuSequencePartner::Fused(fused) => fused.writes,
            SpuSequencePartner::Guest(_) => &[],
        };
        self.sequence
            .iter()
            .chain(partner)
            .flat_map(|word| [word.rt, word.ra, word.rb, word.rc])
            .chain(fused_writes.iter().copied())
            .chain(self.pins.iter().map(|(register, _)| *register))
            .max()
            .map_or(0, |highest| usize::from(highest) + 1)
    }

    /// The real registers a comparison under the dead set `dead` leaves
    /// out: each dead register's, unless a live register sequence A writes
    /// shares it.
    ///
    /// [Bansal2006 p:395 s:2] Equivalence holds under the set of registers
    /// live at exit; a physical register that also holds a live result
    /// stays live.
    pub fn excluded_registers(&self, assignment: &[u8], dead: &[u8]) -> Vec<u8> {
        let real = |symbolic: u8| assignment.get(usize::from(symbolic)).copied();
        let live_written: Vec<u8> = self
            .sequence
            .iter()
            .filter(|word| !dead.contains(&word.rt))
            .filter_map(|word| real(word.rt))
            .collect();
        let mut out: Vec<u8> = dead
            .iter()
            .filter_map(|&symbolic| real(symbolic))
            .filter(|register| !live_written.contains(register))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A two- or three-register word: `kind rt, ra, rb`.
pub(super) const fn rr(kind: SpuInstructionKind, rt: u8, ra: u8, rb: u8) -> SpuSymbolicWord {
    SpuSymbolicWord {
        kind,
        rt,
        ra,
        rb,
        rc: 0,
        imm: 0,
    }
}

/// A four-register word: `kind rt, ra, rb, rc`.
pub(super) const fn rrr(
    kind: SpuInstructionKind,
    rt: u8,
    ra: u8,
    rb: u8,
    rc: u8,
) -> SpuSymbolicWord {
    SpuSymbolicWord {
        kind,
        rt,
        ra,
        rb,
        rc,
        imm: 0,
    }
}

/// An immediate word: `kind rt, ra, imm`; the field value is `imm`'s low
/// bits.
pub(super) const fn ri(kind: SpuInstructionKind, rt: u8, ra: u8, imm: i32) -> SpuSymbolicWord {
    SpuSymbolicWord {
        kind,
        rt,
        ra,
        rb: 0,
        rc: 0,
        imm: imm as u32,
    }
}

/// A relative branch at word `index` of its program, testing `rt`, whose
/// target is the taken landing.
pub(super) const fn branch_to_landing(
    kind: SpuInstructionKind,
    rt: u8,
    index: u32,
) -> SpuSymbolicWord {
    SpuSymbolicWord {
        kind,
        rt,
        ra: 0,
        rb: 0,
        rc: 0,
        imm: (SEQUENCE_TAKEN_LANDING - SEQUENCE_PROGRAM_BASE) / 4 - index,
    }
}

/// True when no two of `registers` share a real register under `assignment`.
pub(super) fn distinct(assignment: &[u8], registers: &[u8]) -> bool {
    registers.iter().enumerate().all(|(index, &left)| {
        registers[index + 1..]
            .iter()
            .all(|&right| assignment[usize::from(left)] != assignment[usize::from(right)])
    })
}

/// True when none of `outputs` shares a real register with any of `inputs`.
pub(super) fn apart(assignment: &[u8], outputs: &[u8], inputs: &[u8]) -> bool {
    outputs.iter().all(|&output| {
        inputs
            .iter()
            .all(|&input| assignment[usize::from(output)] != assignment[usize::from(input)])
    })
}
