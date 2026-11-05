//! The documented PPU reference sets: the branches, `sc` and `mftb`,
//! whose ends a table-probe capture cannot record. A branch's end is its
//! new PC, LR and CTR, which the probe routine's own addresses would
//! carry. `sc` stops at the system call.
//!
//! Each vector's expected end follows from the rule its citation names,
//! through the small model below. Every register a rule leaves alone
//! keeps its start value. Two `bcctr` vectors follow the console instead,
//! where the document leaves the form invalid.

use super::capture::{
    capture_source, unobserved_stop, unsupported, value, DATA_BYTES, ENTRY_VSCR, MEMORY_BASE,
};
use crate::ppu_reference::{
    PpuReferenceInputState, PpuReferenceObservation, PpuReferenceProvenance, PpuReferenceSet,
    PpuReferenceState, PpuReferenceStateBase, PpuReferenceUnit, PpuReferenceVector,
    PpuReferenceYieldReason, ReferenceField, PPU_REFERENCE_SET_SCHEMA_VERSION,
};
use cellgov_testkit::console_cases::PROFILE;

/// [PPC-Book1 p:24 s:2.4.1] `b`: the target is `EXTS(LI || 0b00)`, plus
/// the instruction's address when AA=0, and LK=1 puts the address of the
/// next instruction in LR.
const B: &str = "PPC-Book1 p:24 s:2.4.1";

/// [PPC-Book1 p:24 s:2.4.1] `bc`: BO2=0 decrements CTR, the branch is
/// taken when the CTR test and the CR-bit test BO selects both pass, and
/// the target is `EXTS(BD || 0b00)` as `b` forms it.
const BC: &str = "PPC-Book1 p:24 s:2.4.1";

/// [PPC-Book1 p:25 s:2.4.1] `bclr` branches to `LR[0:61] || 0b00` under
/// the tests `bc` makes, and `bcctr` to `CTR[0:61] || 0b00` under the CR
/// test alone; LK=1 puts the address of the next instruction in LR.
const BCLR_BCCTR: &str = "PPC-Book1 p:25 s:2.4.1";

/// [PPC-Book1 p:26 s:2.4.2] `sc` calls the system to perform a service;
/// the LEV field is the instruction's bits 20:26.
const SC: &str = "PPC-Book1 p:26 s:2.4.2";

/// [PPC-Book2 p:30 s:4.1] `mftb` with TBR 268 reads the time base, and
/// with TBR 269 the time base's upper word, zero-extended.
const MFTB: &str = "PPC-Book2 p:30 s:4.1";

/// The units this module fills.
pub(super) const DOCUMENTED_UNITS: [&str; 6] = ["b", "bc", "bclr", "bcctr", "sc", "mftb"];

/// `ori r0, r0, 0`, the word before a branch that starts at PC 4.
const NOP: u32 = 24 << 26;

/// The CR with bit 2 set: the BI every vector tests.
const CR_EQ: u32 = 0x2000_0000;

/// The start a vector's words run from.
#[derive(Debug, Clone, Copy)]
struct Start {
    pc: u64,
    cr: u32,
    lr: u64,
    ctr: u64,
}

/// What a branch leaves: its new PC, LR and CTR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct End {
    pub(super) pc: u64,
    pub(super) lr: u64,
    pub(super) ctr: u64,
}

/// Bit `n` of a BO value, BO0 its most significant bit.
fn bo_bit(bo: u32, n: u32) -> bool {
    (bo >> (4 - n)) & 1 == 1
}

fn exts(v: u64, bits: u32) -> u64 {
    let shift = 64 - bits;
    (((v << shift) as i64) >> shift) as u64
}

/// The end of one branch word from the given PC, CR, LR and CTR, by the
/// cited rules.
pub(super) fn branch_end(word: u32, pc: u64, cr: u32, lr: u64, ctr: u64) -> End {
    let primary = word >> 26;
    let lk = word & 1 == 1;
    let aa = (word >> 1) & 1 == 1;
    let bo = (word >> 21) & 31;
    let bi = (word >> 16) & 31;
    let next = pc.wrapping_add(4);
    let new_lr = if lk { next } else { lr };
    let relative = |offset: u64| if aa { offset } else { pc.wrapping_add(offset) };
    if primary == 18 {
        let li = exts(u64::from(word & 0x03ff_fffc), 26);
        return End {
            pc: relative(li),
            lr: new_lr,
            ctr,
        };
    }
    let cond_ok = bo_bit(bo, 0) || (((cr >> (31 - bi)) & 1 == 1) == bo_bit(bo, 1));
    let xo = (word >> 1) & 0x3ff;
    if primary == 19 && xo == 528 {
        let target = ctr & !3;
        return End {
            pc: if cond_ok { target } else { next },
            lr: new_lr,
            ctr,
        };
    }
    let new_ctr = if bo_bit(bo, 2) {
        ctr
    } else {
        ctr.wrapping_sub(1)
    };
    let ctr_ok = bo_bit(bo, 2) || ((new_ctr != 0) != bo_bit(bo, 3));
    let target = if primary == 16 {
        relative(exts(u64::from(word & 0xfffc), 16))
    } else {
        lr & !3
    };
    End {
        pc: if ctr_ok && cond_ok { target } else { next },
        lr: new_lr,
        ctr: new_ctr,
    }
}

fn b(offset: i64, aa: bool, lk: bool) -> u32 {
    (18 << 26) | (offset as u32 & 0x03ff_fffc) | (u32::from(aa) << 1) | u32::from(lk)
}

fn bc(bo: u32, offset: i64, aa: bool, lk: bool) -> u32 {
    (16 << 26)
        | (bo << 21)
        | (2 << 16)
        | (offset as u32 & 0xfffc)
        | (u32::from(aa) << 1)
        | u32::from(lk)
}

fn bclr(bo: u32, lk: bool) -> u32 {
    (19 << 26) | (bo << 21) | (2 << 16) | (16 << 1) | u32::from(lk)
}

fn bcctr(bo: u32, lk: bool) -> u32 {
    (19 << 26) | (bo << 21) | (2 << 16) | (528 << 1) | u32::from(lk)
}

/// A 10-bit SPR or TBR number in the swapped-halves field of a word.
fn split_field(n: u32) -> u32 {
    ((n & 31) << 16) | ((n >> 5) << 11)
}

/// The BO values a conditional branch takes: branch always, each CR
/// test, each CTR test, and each CTR test with each CR test.
const BOS: [u32; 9] = [20, 12, 4, 16, 18, 8, 0, 10, 2];

/// The BO values with BO2 = 1, the ones `bcctr` defines.
const CTR_FREE_BOS: [u32; 3] = [20, 12, 4];

fn input(start: Start, gpr: &[(u8, u64)]) -> PpuReferenceInputState {
    PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: gpr.iter().copied().collect(),
        fpr: Default::default(),
        vr_hex: Default::default(),
        pc: start.pc,
        cr: start.cr,
        lr: start.lr,
        ctr: start.ctr,
        xer: 0,
        fpscr: Some(0),
        vrsave: 0,
        vscr: Some(ENTRY_VSCR),
        tb: 0,
        reservation: None,
    }
}

/// The expected end of a vector whose words change only PC, LR and CTR,
/// with `gpr` as its GPR bank.
fn expected(
    start: Start,
    end: End,
    gpr: ReferenceField<Vec<u64>>,
    retired: u64,
) -> PpuReferenceObservation {
    const RUNTIME: &str = "the documented vector does not represent the runtime's effect pipeline";
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr,
            fpr: value(vec![0; 32]),
            vr_hex: value(vec!["0".repeat(32); 32]),
            pc: value(end.pc),
            cr: value(start.cr),
            lr: value(end.lr),
            ctr: value(end.ctr),
            xer: value(0),
            fpscr: value(0),
            vrsave: value(0),
            vscr: value(ENTRY_VSCR),
            tb: ReferenceField::Undefined {
                reason: "the time base advances while the vector runs".to_owned(),
            },
            reservation: value(None),
        },
        memory: value(vec![0; DATA_BYTES]),
        stop: unobserved_stop(),
        retired: value(retired),
        staged_effects: unsupported(RUNTIME),
        committed_effects: unsupported(RUNTIME),
        reservations: unsupported(RUNTIME),
        store_buffer: unsupported(RUNTIME),
        commit_error: unsupported(RUNTIME),
        fault_discarded: unsupported(RUNTIME),
    }
}

fn documented(citation: &str, name: &str) -> PpuReferenceProvenance {
    PpuReferenceProvenance::DocumentedVector {
        citation: citation.to_owned(),
        vector_id: name.to_owned(),
    }
}

fn captured(probe: &str) -> PpuReferenceProvenance {
    let source = capture_source(probe);
    PpuReferenceProvenance::HardwareCapture {
        capture_id: format!("{probe}/{PROFILE}"),
        device: source.device,
        environment: source.environment,
        source_sha256: source.frame_sha256,
    }
}

/// One branch vector: `word` alone from PC 0, or after a no-op from PC 4.
///
/// The reference runs one step per word, over the words and the no-ops
/// after them. A vector from PC 4 thus runs a second step at the branch
/// target. That target must lie in this code:
///
/// - the no-op at 0;
/// - the branch at 4;
/// - a no-op at 8 or 12.
fn branch_vector(
    name: String,
    provenance: PpuReferenceProvenance,
    word: u32,
    start: Start,
    end: End,
    retired: u64,
) -> PpuReferenceVector {
    let words = if start.pc == 4 {
        vec![NOP, word]
    } else {
        vec![word]
    };
    PpuReferenceVector {
        name,
        provenance,
        words,
        initial_state: input(start, &[]),
        memory_base: MEMORY_BASE,
        initial_memory: vec![0; DATA_BYTES],
        expected: expected(start, end, value(vec![0; 32]), retired),
    }
}

/// The end of `word` from `start` by the model, and the steps that ran.
fn model_end(name: &str, word: u32, start: Start) -> (End, u64) {
    let first = branch_end(word, start.pc, start.cr, start.lr, start.ctr);
    if start.pc == 0 {
        return (first, 1);
    }
    assert!(
        first.pc < 16 && first.pc.is_multiple_of(4),
        "{name}: a branch from PC 4 leaves its code at {:#x}",
        first.pc
    );
    let second = if first.pc == 4 {
        branch_end(word, first.pc, start.cr, first.lr, first.ctr)
    } else {
        End {
            pc: first.pc + 4,
            ..first
        }
    };
    (second, 2)
}

fn model_vector(name: String, citation: &str, word: u32, start: Start) -> PpuReferenceVector {
    let (end, retired) = model_end(&name, word, start);
    let provenance = documented(citation, &name);
    branch_vector(name, provenance, word, start, end, retired)
}

const AT_0: Start = Start {
    pc: 0,
    cr: 0,
    lr: 0,
    ctr: 0,
};

const AT_4: Start = Start { pc: 4, ..AT_0 };

/// `b` from PC 0 at the offset extremes, and from PC 4 to targets in its
/// code: AA=1 to 0, 4 and 8, and AA=0 at offsets -4, 0, 4 and 8.
fn b_vectors() -> Vec<PpuReferenceVector> {
    let mut out = Vec::new();
    let cases = [
        (AT_0, 0x01ff_fffc_i64),
        (AT_0, -0x0200_0000),
        (AT_4, 0),
        (AT_4, 4),
        (AT_4, 8),
        (AT_4, -4),
    ];
    for (start, offset) in cases {
        for aa in [false, true] {
            if aa && offset < 0 {
                continue;
            }
            for lk in [false, true] {
                let name = format!(
                    "b-pc{}-{offset:x}-aa{}-lk{}",
                    start.pc,
                    u8::from(aa),
                    u8::from(lk)
                );
                out.push(model_vector(name, B, b(offset, aa, lk), start));
            }
        }
    }
    out
}

/// The CTR values a CTR test meets: one that the decrement takes to
/// zero, one it leaves nonzero, and zero, which it wraps.
const CTRS: [u64; 3] = [1, 2, 0];

/// The starts for each BO value in `bos`, with the CR bit clear and set:
///
/// - a BO that decrements CTR takes each [`CTRS`] start, and LK
///   alternates across them;
/// - any other BO takes `ctr` with each LK.
fn conditional_starts(bos: &[u32], lr: u64, ctr: u64) -> Vec<(u32, Start, bool)> {
    let mut out = Vec::new();
    for &bo in bos {
        for cr in [0, CR_EQ] {
            let ctrs: Vec<u64> = if bo_bit(bo, 2) {
                vec![ctr]
            } else {
                CTRS.to_vec()
            };
            for (k, &c) in ctrs.iter().enumerate() {
                let lks: &[bool] = if bo_bit(bo, 2) {
                    &[false, true]
                } else {
                    &[k % 2 == 1]
                };
                for &lk in lks {
                    out.push((
                        bo,
                        Start {
                            pc: 0,
                            cr,
                            lr,
                            ctr: c,
                        },
                        lk,
                    ));
                }
            }
        }
    }
    out
}

fn bc_vectors() -> Vec<PpuReferenceVector> {
    let mut out = Vec::new();
    for (bo, start, lk) in conditional_starts(&BOS, 0, 5) {
        let name = format!(
            "bc-bo{bo}-cr{}-ctr{}-lk{}",
            u8::from(start.cr != 0),
            start.ctr,
            u8::from(lk)
        );
        out.push(model_vector(name, BC, bc(bo, 0x40, false, lk), start));
    }
    for (offset, aa) in [
        (0_i64, true),
        (8, true),
        (4, false),
        (8, false),
        (-4, false),
    ] {
        let name = format!("bc-always-pc4-{offset:x}-aa{}", u8::from(aa));
        out.push(model_vector(name, BC, bc(20, offset, aa, false), AT_4));
    }
    out
}

fn bclr_vectors() -> Vec<PpuReferenceVector> {
    conditional_starts(&BOS, 0x1003, 5)
        .into_iter()
        .map(|(bo, start, lk)| {
            let name = format!(
                "bclr-bo{bo}-cr{}-ctr{}-lk{}",
                u8::from(start.cr != 0),
                start.ctr,
                u8::from(lk)
            );
            model_vector(name, BCLR_BCCTR, bclr(bo, lk), start)
        })
        .collect()
}

/// The console's `bcctr` with BO2 = 0: it decrements CTR, tests the new
/// CTR as `bc` does, and branches to the CTR it started with.
fn bcctr_bo2_end(bo: u32, start: Start) -> End {
    let ctr = start.ctr.wrapping_sub(1);
    let ctr_ok = (ctr != 0) != bo_bit(bo, 3);
    let cond_ok = bo_bit(bo, 0) || (((start.cr >> 29) & 1 == 1) == bo_bit(bo, 1));
    End {
        pc: if ctr_ok && cond_ok { start.ctr & !3 } else { 4 },
        lr: start.lr,
        ctr,
    }
}

fn bcctr_vectors() -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = conditional_starts(&CTR_FREE_BOS, 0, 0x1003)
        .into_iter()
        .map(|(bo, start, lk)| {
            let name = format!(
                "bcctr-bo{bo}-cr{}-lk{}",
                u8::from(start.cr != 0),
                u8::from(lk)
            );
            model_vector(name, BCLR_BCCTR, bcctr(bo, lk), start)
        })
        .collect();
    // [PPC-Book1 p:25 s:2.4.1] BO2 = 0 makes the bcctr form invalid; the
    // two console captures settle what the PPE does with it.
    for (probe, bo) in [
        ("ppu_may_trap_bcctr_bo2_taken", 8),
        ("ppu_may_trap_bcctr_bo2_ctr", 10),
    ] {
        let start = Start {
            pc: 0,
            cr: CR_EQ,
            lr: 0,
            ctr: 0x40,
        };
        let end = bcctr_bo2_end(bo, start);
        out.push(branch_vector(
            format!("bcctr-bo{bo}-decrements-ctr"),
            captured(probe),
            bcctr(bo, false),
            start,
            end,
            1,
        ));
    }
    out
}

/// `sc` from PC 0 with the syscall registers set: the reference stops at
/// the call with LEV 0, the `sc` retired, and every register as it was.
fn sc_vectors() -> Vec<PpuReferenceVector> {
    let gpr: Vec<(u8, u64)> = (3..=11).map(|n| (n, 0x1111 * u64::from(n))).collect();
    let mut bank = vec![0; 32];
    for &(n, v) in &gpr {
        bank[usize::from(n)] = v;
    }
    [(0, 0), (CR_EQ, 0x40)]
        .into_iter()
        .map(|(cr, lr)| {
            let start = Start {
                pc: 0,
                cr,
                lr,
                ctr: 0,
            };
            let name = format!("sc-lev0-cr{}", u8::from(cr != 0));
            let mut expected = expected(
                start,
                End {
                    pc: 0,
                    lr,
                    ctr: 0,
                },
                value(bank.clone()),
                1,
            );
            expected.state.pc = unsupported(
                "the reference stops at the call, before the system service returns to the next instruction",
            );
            expected.stop.reason = value(PpuReferenceYieldReason::Syscall);
            expected.stop.syscall_lev = value(Some(0));
            expected.stop.pc = value(Some(0));
            PpuReferenceVector {
                provenance: documented(SC, &name),
                name,
                words: vec![(17 << 26) | 2],
                initial_state: input(start, &gpr),
                memory_base: MEMORY_BASE,
                initial_memory: vec![0; DATA_BYTES],
                expected,
            }
        })
        .collect()
}

/// A time-base read into r5 by `mftb` and by `mfspr` with the same
/// number. The value read depends on when the read runs; everything
/// else stays as it was.
fn mftb_vectors() -> Vec<PpuReferenceVector> {
    let mut out = Vec::new();
    for (tbr, upper) in [(268, "tb"), (269, "tbu")] {
        for (xo, form, provenance) in [
            (371, "mftb", documented(MFTB, &format!("mftb-{upper}"))),
            (339, "mfspr", captured(&format!("ppu_may_trap_mfspr{tbr}"))),
        ] {
            let start = Start {
                pc: 0,
                cr: 0,
                lr: 0,
                ctr: 0,
            };
            let gpr = ReferenceField::Undefined {
                reason: "r5 holds the time base, which depends on when the read runs".to_owned(),
            };
            let end = End {
                pc: 4,
                lr: 0,
                ctr: 0,
            };
            out.push(PpuReferenceVector {
                name: format!("{form}-{upper}"),
                provenance,
                words: vec![(31 << 26) | (5 << 21) | split_field(tbr) | (xo << 1)],
                initial_state: input(start, &[]),
                memory_base: MEMORY_BASE,
                initial_memory: vec![0; DATA_BYTES],
                expected: expected(start, end, gpr, 1),
            });
        }
    }
    out
}

/// The set this module writes for `unit`.
pub(super) fn documented_set(unit: &str) -> PpuReferenceSet {
    let vectors = match unit {
        "b" => b_vectors(),
        "bc" => bc_vectors(),
        "bclr" => bclr_vectors(),
        "bcctr" => bcctr_vectors(),
        "sc" => sc_vectors(),
        "mftb" => mftb_vectors(),
        other => panic!("{other} is not a documented unit"),
    };
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors,
    }
}
