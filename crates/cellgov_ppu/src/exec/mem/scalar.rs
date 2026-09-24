//! The scalar load and store model, its classifier, and the one load and one store it runs through.

use crate::exec::memory_helpers::{buffer_store, load_se, load_ze, LoadPort, Width};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

use super::helpers::{double_word, single_frs, swap_low_bytes};

/// The effective-address form of a scalar load or store.
#[derive(Clone, Copy)]
pub(super) enum Ea {
    /// `(RA|0) + EXTS(D)`; the decoder clears the low two bits of a
    /// DS-form offset.
    D(u8, i16),
    /// `(RA|0) + (RB)`.
    X(u8, u8),
}

impl Ea {
    #[inline]
    fn resolve(self, state: &PpuState) -> u64 {
        match self {
            Ea::D(ra, imm) => state.ea_d_form(ra, imm),
            Ea::X(ra, rb) => state.ea_x_form(ra, rb),
        }
    }

    /// The base register an update form writes EA back to.
    pub(super) fn ra(self) -> u8 {
        match self {
            Ea::D(ra, _) | Ea::X(ra, _) => ra,
        }
    }
}

/// Where a scalar load lands, and how it widens the bytes on the way.
#[derive(Clone, Copy)]
pub(super) enum Dest {
    /// Zero-extended into a GPR.
    Zero(u8),
    /// Sign-extended into a GPR.
    Sign(u8),
    /// Byte-reversed, then zero-extended into a GPR.
    Reversed(u8),
    /// Into an FPR: a word goes through `DOUBLE`, a doubleword lands
    /// verbatim.
    Fpr(u8),
}

/// What a scalar store writes; the store buffer keeps the low `width`
/// bytes of the value.
#[derive(Clone, Copy)]
pub(super) enum Src {
    Gpr(u8),
    /// A GPR, byte-reversed within the stored width.
    Reversed(u8),
    /// An FPR: a word goes through `SINGLE`, a doubleword passes
    /// verbatim.
    Fpr(u8),
    /// An FPR's raw bits with no conversion at any width.
    FprBits(u8),
}

/// Whether the instruction writes EA to RA once the access succeeds.
#[derive(Clone, Copy)]
pub(super) enum Update {
    No,
    /// Write EA to RA; the mnemonic names the instruction in the
    /// invalid-form fault.
    Ra(&'static str),
}

/// A scalar load or store, split on the four axes its forms differ by:
///
/// - effective-address form
/// - width
/// - extension / register file
/// - base-register update
#[derive(Clone, Copy)]
pub(super) enum Scalar {
    Load(Ea, Width, Dest, Update),
    Store(Ea, Width, Src, Update),
}

/// Decode the scalar load / store forms; `None` for every other
/// memory instruction.
#[inline]
pub(super) fn scalar(insn: &PpuInstruction) -> Option<Scalar> {
    use Ea::{D, X};
    use Scalar::{Load, Store};
    use Update::{No, Ra};
    use Width::{B1, B2, B4, B8};
    Some(match *insn {
        // Integer loads
        // [PPC-Book1 p:34 s:3.3] Load Byte and Zero (lbz, D-form): byte at EA -> RT[56:63], RT[0:55]=0.
        // [PPC-Book1 p:35 s:3.3] Load Halfword and Zero (lhz, D-form): zero-extend halfword to RT.
        // [PPC-Book1 p:36 s:3.3] Load Halfword Algebraic (lha, D-form): sign-extend halfword to RT.
        // [PPC-Book1 p:38 s:3.3] Load Word Algebraic (lwa, DS-form): sign-extend word to RT.
        // [PPC-Book1 p:39 s:3.3] Load Doubleword (ld, DS-form): MEM(EA,8) -> RT.
        PpuInstruction::Lwz { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Zero(rt), No),
        PpuInstruction::Lbz { rt, ra, imm } => Load(D(ra, imm), B1, Dest::Zero(rt), No),
        PpuInstruction::Lhz { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Zero(rt), No),
        PpuInstruction::Lha { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Sign(rt), No),
        // [PPC-Book1 p:36 s:3.3] lhau D-form: sign-extend halfword to RT; RA = EA. Requires RA != 0 && RA != RT.
        PpuInstruction::Lhau { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Sign(rt), Ra("lhau")),
        PpuInstruction::Lwzu { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Zero(rt), Ra("lwzu")),
        PpuInstruction::Lbzu { rt, ra, imm } => Load(D(ra, imm), B1, Dest::Zero(rt), Ra("lbzu")),
        PpuInstruction::Lhzu { rt, ra, imm } => Load(D(ra, imm), B2, Dest::Zero(rt), Ra("lhzu")),
        PpuInstruction::Ldu { rt, ra, imm } => Load(D(ra, imm), B8, Dest::Zero(rt), Ra("ldu")),
        PpuInstruction::Ld { rt, ra, imm } => Load(D(ra, imm), B8, Dest::Zero(rt), No),
        PpuInstruction::Lwa { rt, ra, imm } => Load(D(ra, imm), B4, Dest::Sign(rt), No),
        // [PPC-Book1 p:34 s:3.3] X-form indexed load variants (lbzx/lhzx/lwzx/ldx): EA = (RA|0)+(RB).
        PpuInstruction::Lwzx { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Zero(rt), No),
        PpuInstruction::Lbzx { rt, ra, rb } => Load(X(ra, rb), B1, Dest::Zero(rt), No),
        PpuInstruction::Ldx { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Zero(rt), No),
        PpuInstruction::Lhzx { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Zero(rt), No),
        // [PPC-Book1 p:34 s:3.3.1] X-form indexed loads with update: EA = (RA|0)+(RB), then RA = EA.
        PpuInstruction::Lwzux { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Zero(rt), Ra("lwzux")),
        PpuInstruction::Lbzux { rt, ra, rb } => Load(X(ra, rb), B1, Dest::Zero(rt), Ra("lbzux")),
        PpuInstruction::Lhzux { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Zero(rt), Ra("lhzux")),
        PpuInstruction::Ldux { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Zero(rt), Ra("ldux")),
        // [PPC-Book1 p:36 s:3.3] lhax / lhaux: load halfword algebraic (sign-extend 16->64).
        PpuInstruction::Lhax { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Sign(rt), No),
        PpuInstruction::Lhaux { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Sign(rt), Ra("lhaux")),
        // [PPC-Book1 p:38 s:3.3] lwax / lwaux: load word algebraic (sign-extend 32->64).
        PpuInstruction::Lwax { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Sign(rt), No),
        PpuInstruction::Lwaux { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Sign(rt), Ra("lwaux")),

        // Integer stores
        // [PPC-Book1 p:40 s:3.3.3] Store Byte (stb, D-form): RS[56:63] -> MEM(EA,1).
        // [PPC-Book1 p:41 s:3.3.3] Store Halfword (sth, D-form): RS[48:63] -> MEM(EA,2).
        // [PPC-Book1 p:42 s:3.3.3] Store Word (stw/stwx/stwu, D/X-form): RS[32:63] -> MEM(EA,4).
        // [PPC-Book1 p:43 s:3.3.3] Store Doubleword (std/stdx, DS/X-form): RS -> MEM(EA,8).
        PpuInstruction::Stw { rs, ra, imm } => Store(D(ra, imm), B4, Src::Gpr(rs), No),
        PpuInstruction::Stb { rs, ra, imm } => Store(D(ra, imm), B1, Src::Gpr(rs), No),
        PpuInstruction::Stbu { rs, ra, imm } => Store(D(ra, imm), B1, Src::Gpr(rs), Ra("stbu")),
        PpuInstruction::Sth { rs, ra, imm } => Store(D(ra, imm), B2, Src::Gpr(rs), No),
        PpuInstruction::Sthu { rs, ra, imm } => Store(D(ra, imm), B2, Src::Gpr(rs), Ra("sthu")),
        PpuInstruction::Std { rs, ra, imm } => Store(D(ra, imm), B8, Src::Gpr(rs), No),
        PpuInstruction::Stwu { rs, ra, imm } => Store(D(ra, imm), B4, Src::Gpr(rs), Ra("stwu")),
        PpuInstruction::Stdu { rs, ra, imm } => Store(D(ra, imm), B8, Src::Gpr(rs), Ra("stdu")),
        PpuInstruction::Stwx { rs, ra, rb } => Store(X(ra, rb), B4, Src::Gpr(rs), No),
        PpuInstruction::Stdx { rs, ra, rb } => Store(X(ra, rb), B8, Src::Gpr(rs), No),
        PpuInstruction::Stdux { rs, ra, rb } => Store(X(ra, rb), B8, Src::Gpr(rs), Ra("stdux")),
        PpuInstruction::Stbx { rs, ra, rb } => Store(X(ra, rb), B1, Src::Gpr(rs), No),
        // [PPC-Book1 p:41 s:3.3.3] sthx X-form: low halfword of RS -> MEM(EA, 2).
        PpuInstruction::Sthx { rs, ra, rb } => Store(X(ra, rb), B2, Src::Gpr(rs), No),
        // [PPC-Book1 p:41 s:3.3.3] sthux X-form with update; RA != 0.
        PpuInstruction::Sthux { rs, ra, rb } => Store(X(ra, rb), B2, Src::Gpr(rs), Ra("sthux")),
        // [PPC-Book1 p:42 s:3.3.3] stwux X-form with update; RA != 0.
        PpuInstruction::Stwux { rs, ra, rb } => Store(X(ra, rb), B4, Src::Gpr(rs), Ra("stwux")),
        // [PPC-Book1 p:40 s:3.3.3] stbux X-form with update; RA != 0.
        PpuInstruction::Stbux { rs, ra, rb } => Store(X(ra, rb), B1, Src::Gpr(rs), Ra("stbux")),

        // Byte-reverse indexed loads and stores
        // [PPC-Book1 p:50 s:3.3.4] lwbrx / lhbrx: load size N, low N bytes byte-reversed, zero-extended into RT.
        // [PPC-Book1 p:51 s:3.3.4] ldbrx / stwbrx / sthbrx: doubleword load / word + halfword store with byte reversal.
        // [CBE-Handbook p:734 s:A.2.1] sdbrx (CG name): low-64 byte-reverse store.
        PpuInstruction::Ldbrx { rt, ra, rb } => Load(X(ra, rb), B8, Dest::Reversed(rt), No),
        PpuInstruction::Lwbrx { rt, ra, rb } => Load(X(ra, rb), B4, Dest::Reversed(rt), No),
        PpuInstruction::Lhbrx { rt, ra, rb } => Load(X(ra, rb), B2, Dest::Reversed(rt), No),
        PpuInstruction::Sdbrx { rs, ra, rb } => Store(X(ra, rb), B8, Src::Reversed(rs), No),
        PpuInstruction::Stwbrx { rs, ra, rb } => Store(X(ra, rb), B4, Src::Reversed(rs), No),
        PpuInstruction::Sthbrx { rs, ra, rb } => Store(X(ra, rb), B2, Src::Reversed(rs), No),

        // Floating-point loads / stores
        // [PPC-Book1 p:104 s:4.6] Load Floating-Point Single (lfs, D-form): single -> double via DOUBLE() into FRT.
        // [PPC-Book1 p:105 s:4.6] Load Floating-Point Double (lfd, D-form): MEM(EA,8) -> FRT.
        // [PPC-Book1 p:107 s:4.6] Store Floating-Point Single (stfs, D-form): SINGLE(FRS) -> MEM(EA,4).
        // [PPC-Book1 p:108 s:4.6] Store Floating-Point Double (stfd, D-form): FRS -> MEM(EA,8).
        PpuInstruction::Lfs { frt, ra, imm } => Load(D(ra, imm), B4, Dest::Fpr(frt), No),
        // [PPC-Book1 p:104 s:4.6.2] lfsu D-form: lfs with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfsu { frt, ra, imm } => Load(D(ra, imm), B4, Dest::Fpr(frt), Ra("lfsu")),
        PpuInstruction::Lfd { frt, ra, imm } => Load(D(ra, imm), B8, Dest::Fpr(frt), No),
        // [PPC-Book1 p:105 s:4.6.2] lfdu D-form: lfd with EA written to RA. Requires RA != 0.
        PpuInstruction::Lfdu { frt, ra, imm } => Load(D(ra, imm), B8, Dest::Fpr(frt), Ra("lfdu")),
        PpuInstruction::Stfs { frs, ra, imm } => Store(D(ra, imm), B4, Src::Fpr(frs), No),
        PpuInstruction::Stfd { frs, ra, imm } => Store(D(ra, imm), B8, Src::Fpr(frs), No),
        PpuInstruction::Stfsu { frs, ra, imm } => Store(D(ra, imm), B4, Src::Fpr(frs), Ra("stfsu")),
        PpuInstruction::Stfdu { frs, ra, imm } => Store(D(ra, imm), B8, Src::Fpr(frs), Ra("stfdu")),
        // Unlike stfs, stfiwx stores the low 32 FPR bits verbatim
        // (no round-convert to single precision).
        // [PPC-Book1 p:109 s:4.6] Store Floating-Point as Integer Word Indexed (stfiwx): FRS[32:63] -> MEM(EA,4) without conversion.
        PpuInstruction::Stfiwx { frs, ra, rb } => Store(X(ra, rb), B4, Src::FprBits(frs), No),
        // X-form FP indexed loads / stores. EA = (RA == 0 ? 0 : GPR[RA]) + GPR[RB].
        // The `u` (update) variants write EA back into GPR[RA] iff the
        // memory access succeeded, matching the D-form Stfsu/Stfdu policy.
        // [PPC-Book1 p:104 s:4.6] lfsx / lfsux: X-form single-precision load with DOUBLE() conversion.
        // [PPC-Book1 p:105 s:4.6] lfdx / lfdux: X-form double-precision load.
        // [PPC-Book1 p:107 s:4.6] stfsx / stfsux: X-form single-precision store via SINGLE() conversion.
        // [PPC-Book1 p:108 s:4.6] stfdx / stfdux: X-form double-precision store.
        PpuInstruction::Lfsx { frt, ra, rb } => Load(X(ra, rb), B4, Dest::Fpr(frt), No),
        PpuInstruction::Lfsux { frt, ra, rb } => Load(X(ra, rb), B4, Dest::Fpr(frt), Ra("lfsux")),
        PpuInstruction::Lfdx { frt, ra, rb } => Load(X(ra, rb), B8, Dest::Fpr(frt), No),
        PpuInstruction::Lfdux { frt, ra, rb } => Load(X(ra, rb), B8, Dest::Fpr(frt), Ra("lfdux")),
        PpuInstruction::Stfsx { frs, ra, rb } => Store(X(ra, rb), B4, Src::Fpr(frs), No),
        PpuInstruction::Stfsux { frs, ra, rb } => Store(X(ra, rb), B4, Src::Fpr(frs), Ra("stfsux")),
        PpuInstruction::Stfdx { frs, ra, rb } => Store(X(ra, rb), B8, Src::Fpr(frs), No),
        PpuInstruction::Stfdux { frs, ra, rb } => Store(X(ra, rb), B8, Src::Fpr(frs), Ra("stfdux")),
        _ => return None,
    })
}

/// Runs one scalar load, then writes EA to RA for an update form.
///
/// The writeback follows a successful load only, so a fault leaves RA
/// intact.
#[inline]
pub(super) fn load(
    state: &mut PpuState,
    port: &mut LoadPort<'_, '_>,
    ea: Ea,
    width: Width,
    dest: Dest,
    update: Update,
) -> ExecuteVerdict {
    if let Update::Ra(insn) = update {
        let ra = ea.ra();
        let invalid = match dest {
            // [PPC-Book1 p:33 s:3.3.2] Fixed-point load with update: invalid form when RA=0 or RA=RT.
            Dest::Zero(rt) | Dest::Sign(rt) | Dest::Reversed(rt) => ra == 0 || ra == rt,
            // [PPC-Book1 p:104 s:4.6.2] Floating-point load with update: the only invalid form is RA=0.
            // FRT indexes a different register file, so RA=FRT is a valid encoding.
            Dest::Fpr(_) => ra == 0,
        };
        if invalid {
            return ExecuteVerdict::Fault(PpuFault::InvalidForm(insn));
        }
    }
    let addr = ea.resolve(state);
    let loaded = match dest {
        Dest::Sign(_) => load_se(port, addr, width),
        Dest::Zero(_) | Dest::Reversed(_) | Dest::Fpr(_) => load_ze(port, addr, width),
    };
    let val = match loaded {
        Ok(val) => val,
        Err(e) => return ExecuteVerdict::MemFault(e),
    };
    match dest {
        Dest::Zero(rt) | Dest::Sign(rt) => state.set_gpr(rt as usize, val),
        Dest::Reversed(rt) => state.set_gpr(rt as usize, swap_low_bytes(val, width)),
        Dest::Fpr(frt) => state.set_fpr(
            frt as usize,
            match width {
                Width::B4 => double_word(val as u32),
                Width::B1 | Width::B2 | Width::B8 => val,
            },
        ),
    }
    if let Update::Ra(_) = update {
        state.set_gpr(ea.ra() as usize, addr);
    }
    ExecuteVerdict::Continue
}

/// Stages one scalar store, then writes EA to RA for an update form.
///
/// The writeback follows a staged store only, so a `BufferFull`
/// verdict leaves RA intact for the retry.
#[inline]
pub(super) fn store(
    state: &mut PpuState,
    store_buf: &mut StoreBuffer,
    ea: Ea,
    width: Width,
    src: Src,
    update: Update,
) -> ExecuteVerdict {
    if let Update::Ra(insn) = update {
        // [PPC-Book1 p:40 s:3.3.3] Store with update: invalid form when RA=0.
        if ea.ra() == 0 {
            return ExecuteVerdict::Fault(PpuFault::InvalidForm(insn));
        }
    }
    let addr = ea.resolve(state);
    let value = match src {
        Src::Gpr(rs) => state.gpr[rs as usize],
        Src::Reversed(rs) => swap_low_bytes(state.gpr[rs as usize], width),
        Src::Fpr(frs) => match width {
            Width::B4 => single_frs(state.fpr[frs as usize]) as u64,
            Width::B1 | Width::B2 | Width::B8 => state.fpr[frs as usize],
        },
        Src::FprBits(frs) => state.fpr[frs as usize],
    };
    let verdict = buffer_store(store_buf, state, addr, width.bytes(), value);
    if let Update::Ra(_) = update {
        if verdict.allows_writeback() {
            state.set_gpr(ea.ra() as usize, addr);
        }
    }
    verdict
}
