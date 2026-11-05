//! The documented PPU reference sets of the load and store multiple and
//! string instructions. A table-probe case cannot run them: their
//! register ranges reach the probe routine's own registers, and they move
//! more bytes than a case records.
//!
//! Each vector's expected end follows from the rule its citation names,
//! through the small model below. The console settles three things the
//! document leaves open, and those vectors carry its capture:
//!
//! - a misaligned `lmw` or `stmw` ends the process;
//! - each invalid string form the captures send ends the process;
//! - `lswx` with a zero count reads one byte.

use super::capture::{
    capture_source, convert, unobserved_stop, unsupported, value, ENTRY_VSCR, MEMORY_BASE,
};
use crate::ppu_reference::{
    PpuReferenceInputState, PpuReferenceObservation, PpuReferenceProvenance, PpuReferenceSet,
    PpuReferenceState, PpuReferenceStateBase, PpuReferenceUnit, PpuReferenceVector,
    PpuReferenceYieldReason, ReferenceField, PPU_REFERENCE_SET_SCHEMA_VERSION,
};
use cellgov_testkit::console_cases::{table, PROFILE};

/// [PPC-Book1 p:46 s:3.3.5] `lmw` loads the words from `(RA|0) + D` into
/// the low halves of RT through r31 and clears their high halves; `stmw`
/// stores the low halves of RS through r31.
const MULTIPLE: &str = "PPC-Book1 p:46 s:3.3.5";

/// [PPC-Book1 p:48 s:3.3.6] `lswi` and `lswx` load their bytes left to
/// right into the low halves of RT onwards, wrap from r31 to r0, clear
/// each high half, and clear the bytes the last register does not fill.
const LOAD_STRING: &str = "PPC-Book1 p:48 s:3.3.6";

/// [PPC-Book1 p:49 s:3.3.6] `stswi` and `stswx` store the bytes of the
/// low halves of RS onwards, left to right; a zero `stswx` count stores
/// nothing.
const STORE_STRING: &str = "PPC-Book1 p:49 s:3.3.6";

/// The units this module fills.
pub(super) const MULTIPLE_UNITS: [&str; 6] = ["lmw", "stmw", "lswi", "lswx", "stswi", "stswx"];

/// The bytes each vector's data window holds.
const WINDOW: usize = 128;

/// The base register each modelled vector names, apart from the RA = 0
/// forms.
const RA: usize = 2;

/// The index register of the indexed string forms, unless the run's
/// range covers it.
const RB: usize = 4;

/// The data window's start bytes: no two adjacent bytes equal, and both
/// values of each byte's top bit.
fn window() -> Vec<u8> {
    (0..WINDOW)
        .map(|i| (i as u8).wrapping_mul(0x3b).wrapping_add(0x91))
        .collect()
}

/// The GPRs a vector starts with: a distinct value per register, with a
/// high half a word move must clear or drop.
fn start_gprs() -> [u64; 32] {
    std::array::from_fn(|n| 0xc3c3_0000_a5a5_0000 | ((n as u64) << 8) | n as u64)
}

fn d_form(op: u32, rt: usize, ra: usize, d: i16) -> u32 {
    (op << 26) | ((rt as u32) << 21) | ((ra as u32) << 16) | u32::from(d as u16)
}

fn x_form(xo: u32, rt: usize, ra: usize, rb: usize) -> u32 {
    (31 << 26) | ((rt as u32) << 21) | ((ra as u32) << 16) | ((rb as u32) << 11) | (xo << 1)
}

/// The string word forms; `nb` is the NB field of the immediate forms
/// and RB of the indexed ones.
fn string(xo: u32, rt: usize, ra: usize, nb: usize) -> u32 {
    x_form(xo, rt, ra, nb)
}

const LSWI: u32 = 597;
const LSWX: u32 = 533;
const STSWI: u32 = 725;
const STSWX: u32 = 661;

/// The registers and data a model step reads and writes.
struct Machine {
    gpr: [u64; 32],
    data: Vec<u8>,
    xer: u64,
}

impl Machine {
    /// The window offset of an effective address.
    fn at(&self, ea: u64) -> usize {
        let at = ea.wrapping_sub(MEMORY_BASE) as usize;
        assert!(
            at < self.data.len(),
            "an access at {ea:#x} leaves the data window"
        );
        at
    }

    fn base(&self, ra: usize) -> u64 {
        if ra == 0 {
            0
        } else {
            self.gpr[ra]
        }
    }

    /// Runs one word by the cited rules.
    fn step(&mut self, word: u32) {
        let rt = ((word >> 21) & 31) as usize;
        let ra = ((word >> 16) & 31) as usize;
        let rb = ((word >> 11) & 31) as usize;
        let d = i64::from(word as u16 as i16) as u64;
        match (word >> 26, (word >> 1) & 0x3ff) {
            (46, _) => self.multiple(rt, self.base(ra).wrapping_add(d), false),
            (47, _) => self.multiple(rt, self.base(ra).wrapping_add(d), true),
            (31, LSWI) => self.string(rt, self.base(ra), if rb == 0 { 32 } else { rb }, false),
            (31, STSWI) => self.string(rt, self.base(ra), if rb == 0 { 32 } else { rb }, true),
            (31, LSWX) => self.string(
                rt,
                self.base(ra).wrapping_add(self.gpr[rb]),
                (self.xer & 0x7f) as usize,
                false,
            ),
            (31, STSWX) => self.string(
                rt,
                self.base(ra).wrapping_add(self.gpr[rb]),
                (self.xer & 0x7f) as usize,
                true,
            ),
            _ => panic!("{word:#010x} is not a multiple or string word"),
        }
    }

    fn multiple(&mut self, first: usize, ea: u64, store: bool) {
        for (k, r) in (first..32).enumerate() {
            let at = self.at(ea + 4 * k as u64);
            if store {
                self.data[at..at + 4].copy_from_slice(&(self.gpr[r] as u32).to_be_bytes());
            } else {
                let w = u32::from_be_bytes(self.data[at..at + 4].try_into().expect("4 bytes"));
                self.gpr[r] = u64::from(w);
            }
        }
    }

    /// Moves `n` bytes from `ea` between memory and the low halves of the
    /// registers from `first`. The range wraps from r31 to r0.
    fn string(&mut self, first: usize, ea: u64, n: usize, store: bool) {
        let mut r = (first + 31) % 32;
        let mut i = 32;
        for k in 0..n {
            if i == 32 {
                r = (r + 1) % 32;
                if !store {
                    self.gpr[r] = 0;
                }
            }
            let shift = 56 - i;
            let at = self.at(ea + k as u64);
            if store {
                self.data[at] = (self.gpr[r] >> shift) as u8;
            } else {
                self.gpr[r] |= u64::from(self.data[at]) << shift;
            }
            i += 8;
            if i == 64 {
                i = 32;
            }
        }
    }
}

/// One documented vector: `word` from the start GPRs with `regs` over
/// them, the window, and `xer`.
fn model_vector(
    name: String,
    citation: &str,
    word: u32,
    regs: &[(usize, u64)],
    xer: u64,
) -> PpuReferenceVector {
    model_vector_over(name, citation, word, regs, xer, window())
}

/// [`model_vector`] over the data window `data`.
pub(super) fn model_vector_over(
    name: String,
    citation: &str,
    word: u32,
    regs: &[(usize, u64)],
    xer: u64,
    data: Vec<u8>,
) -> PpuReferenceVector {
    let mut gpr = start_gprs();
    for &(n, v) in regs {
        gpr[n] = v;
    }
    let start = Machine {
        gpr,
        data: data.clone(),
        xer,
    };
    let mut end = Machine { gpr, data, xer };
    end.step(word);
    PpuReferenceVector {
        provenance: PpuReferenceProvenance::DocumentedVector {
            citation: citation.to_owned(),
            vector_id: name.clone(),
        },
        name,
        words: vec![word],
        initial_state: input(&start),
        memory_base: MEMORY_BASE,
        initial_memory: start.data,
        expected: expected(&end),
    }
}

fn input(m: &Machine) -> PpuReferenceInputState {
    PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: (0..32u8).map(|n| (n, m.gpr[usize::from(n)])).collect(),
        fpr: Default::default(),
        vr_hex: Default::default(),
        pc: 0,
        cr: 0,
        lr: 0,
        ctr: 0,
        xer: m.xer,
        fpscr: Some(0),
        vrsave: 0,
        vscr: Some(ENTRY_VSCR),
        tb: 0,
        reservation: None,
    }
}

const RUNTIME: &str = "the documented vector does not represent the runtime's effect pipeline";

fn expected(m: &Machine) -> PpuReferenceObservation {
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr: value(m.gpr.to_vec()),
            fpr: value(vec![0; 32]),
            vr_hex: value(vec!["0".repeat(32); 32]),
            pc: value(4),
            cr: value(0),
            lr: value(0),
            ctr: value(0),
            xer: value(m.xer),
            fpscr: value(0),
            vrsave: value(0),
            vscr: value(ENTRY_VSCR),
            tb: ReferenceField::Undefined {
                reason: "the time base advances while the vector runs".to_owned(),
            },
            reservation: value(None),
        },
        memory: value(m.data.clone()),
        stop: unobserved_stop(),
        retired: value(1),
        staged_effects: unsupported(RUNTIME),
        committed_effects: unsupported(RUNTIME),
        reservations: unsupported(RUNTIME),
        store_buffer: unsupported(RUNTIME),
        commit_error: unsupported(RUNTIME),
        fault_discarded: unsupported(RUNTIME),
    }
}

/// A [`fault_only_vector`] from a console capture that ended the process.
pub(super) fn ended_vector(
    name: String,
    probe: &str,
    words: Vec<u32>,
    regs: &[(usize, u64)],
    xer: u64,
) -> PpuReferenceVector {
    let source = capture_source(probe);
    let provenance = PpuReferenceProvenance::HardwareCapture {
        capture_id: format!("{probe}/{PROFILE}"),
        device: source.device,
        environment: source.environment,
        source_sha256: source.frame_sha256,
    };
    let reason = "the console run ended the process, so it observed no end state";
    fault_only_vector(name, provenance, reason, words, regs, xer)
}

/// A vector that expects the reference to stop on a fault and marks
/// every other component unsupported for `reason`.
pub(super) fn fault_only_vector(
    name: String,
    provenance: PpuReferenceProvenance,
    reason: &str,
    words: Vec<u32>,
    regs: &[(usize, u64)],
    xer: u64,
) -> PpuReferenceVector {
    let mut gpr = start_gprs();
    for &(n, v) in regs {
        gpr[n] = v;
    }
    let start = Machine {
        gpr,
        data: window(),
        xer,
    };
    let mut stop = unobserved_stop();
    stop.reason = value(PpuReferenceYieldReason::Fault);
    PpuReferenceVector {
        name,
        provenance,
        words,
        initial_state: input(&start),
        memory_base: MEMORY_BASE,
        initial_memory: start.data,
        expected: PpuReferenceObservation {
            state: PpuReferenceState {
                gpr: unsupported(reason),
                fpr: unsupported(reason),
                vr_hex: unsupported(reason),
                pc: unsupported(reason),
                cr: unsupported(reason),
                lr: unsupported(reason),
                ctr: unsupported(reason),
                xer: unsupported(reason),
                fpscr: unsupported(reason),
                vrsave: unsupported(reason),
                vscr: unsupported(reason),
                tb: unsupported(reason),
                reservation: unsupported(reason),
            },
            memory: unsupported(reason),
            stop,
            retired: unsupported(reason),
            staged_effects: unsupported(reason),
            committed_effects: unsupported(reason),
            reservations: unsupported(reason),
            store_buffer: unsupported(reason),
            commit_error: unsupported(reason),
            fault_discarded: unsupported(reason),
        },
    }
}

/// The address `off` bytes into the window.
fn addr(off: u64) -> u64 {
    MEMORY_BASE + off
}

/// `lmw` or `stmw` from RT 31, 28, 20 and 3, with a zero, positive and
/// negative displacement; then each misaligned console capture.
fn multiple_vectors(store: bool) -> Vec<PpuReferenceVector> {
    let (primary, name) = if store { (47, "stmw") } else { (46, "lmw") };
    let op = |rt, ra, d| d_form(primary, rt, ra, d);
    let mut out = Vec::new();
    for (rt, base, d) in [(31, 0, 0), (28, 0, 12), (20, 32, -16), (3, 0, 4)] {
        out.push(model_vector(
            format!("{name}-r{rt}-base{base}-d{d}"),
            MULTIPLE,
            op(rt, RA, d),
            &[(RA, addr(base))],
            0,
        ));
    }
    // [PPC-Book1 p:46 s:3.3.5] The multiple-word EA should be
    // word-aligned.
    // [CBE-Handbook p:253 s:9.5.8] The PPE takes an alignment interrupt
    // when an lmw or stmw operand is not aligned. The console captures
    // settle what the system does with that interrupt. The probes start
    // at r28, with an EA 1, 2 and 3 bytes past a word boundary.
    for k in 1..=3 {
        out.push(ended_vector(
            format!("{name}-r28-misaligned{k}"),
            &format!("ppu_may_trap_{name}_m{k}"),
            vec![op(28, 6, 0)],
            &[(6, addr(16 + k))],
            0,
        ));
    }
    out
}

/// The first register, byte count and data offset of each string run:
///
/// - one byte;
/// - a whole word;
/// - a word and one byte;
/// - 32 bytes;
/// - two ranges that wrap past r31.
const STRING_RUNS: [(usize, usize, u64); 6] = [
    (5, 1, 0),
    (5, 4, 3),
    (5, 5, 1),
    (3, 32, 0),
    (30, 12, 2),
    (31, 7, 0),
];

/// `lswi` or `stswi` over [`STRING_RUNS`], from RA at the run's offset.
fn immediate_vectors(xo: u32, name: &str, citation: &str) -> Vec<PpuReferenceVector> {
    STRING_RUNS
        .iter()
        .map(|&(rt, n, off)| {
            model_vector(
                format!("{name}-r{rt}-n{n}-off{off}"),
                citation,
                string(xo, rt, RA, n % 32),
                &[(RA, addr(off))],
                0,
            )
        })
        .collect()
}

/// `lswx` or `stswx` over [`STRING_RUNS`] with the count in XER, then
/// the RA = 0 form with RB holding the address.
fn indexed_vectors(xo: u32, name: &str, citation: &str) -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = STRING_RUNS
        .iter()
        .map(|&(rt, n, off)| {
            // RB sits outside the range the run loads.
            let rb = if rt == 3 { 11 } else { RB };
            model_vector(
                format!("{name}-r{rt}-n{n}-off{off}"),
                citation,
                string(xo, rt, RA, rb),
                &[(RA, addr(0)), (rb, off)],
                n as u64,
            )
        })
        .collect();
    out.push(model_vector(
        format!("{name}-ra0-n6"),
        citation,
        string(xo, 5, 0, RB),
        &[(RB, addr(9))],
        6,
    ));
    out
}

/// The vector of a one-case console capture that ended the process: the
/// case's own words and inputs, with the buffer at the window.
pub(super) fn ended_case_vector(probe: &str) -> PpuReferenceVector {
    let cases = table(probe);
    let [case] = &cases[..] else {
        panic!("{probe}: one case");
    };
    let mut regs: Vec<(usize, u64)> = case
        .inputs
        .iter()
        .filter_map(|&(r, v)| match r {
            cellgov_testkit::console_cases::Reg::Gpr(n) => Some((usize::from(n), v as u64)),
            _ => None,
        })
        .collect();
    regs.push((
        usize::from(cellgov_testkit::console_cases::MEM_BASE_GPR),
        MEMORY_BASE,
    ));
    let mut v = ended_vector(
        probe.trim_start_matches("ppu_may_trap_").replace('_', "-"),
        probe,
        case.words.clone(),
        &regs,
        case.xer,
    );
    if let Some(mem) = &case.mem {
        let dropped = mem.get(WINDOW..).unwrap_or_default();
        assert!(
            dropped.iter().all(|&b| b == 0),
            "{probe}: memory past the window"
        );
        v.initial_memory[..mem.len().min(WINDOW)].copy_from_slice(&mem[..mem.len().min(WINDOW)]);
    }
    v
}

/// The invalid string forms the console ran. Each one ended the process.
///
/// - RA in the range `lswi` loads;
/// - RA in the range `lswx` loads;
/// - RB in the range `lswx` loads;
/// - RT equal to RA with a zero `lswx` count.
///
/// The capture with a zero count at address 0 is absent: the reference
/// places its code at address 0.
const INVALID_STRING_PROBES: [(&str, &str); 4] = [
    ("lswi", "ppu_may_trap_lswi_ra"),
    ("lswx", "ppu_may_trap_lswx_ra"),
    ("lswx", "ppu_may_trap_lswx_rb"),
    ("lswx", "ppu_may_trap_lswx_rt_ra_n0"),
];

/// The set this module writes for `unit`.
pub(super) fn multiple_set(unit: &str) -> PpuReferenceSet {
    let mut vectors = match unit {
        "lmw" => multiple_vectors(false),
        "stmw" => multiple_vectors(true),
        "lswi" => immediate_vectors(LSWI, "lswi", LOAD_STRING),
        "stswi" => immediate_vectors(STSWI, "stswi", STORE_STRING),
        "lswx" => indexed_vectors(LSWX, "lswx", LOAD_STRING),
        "stswx" => {
            let mut out = indexed_vectors(STSWX, "stswx", STORE_STRING);
            out.push(model_vector(
                "stswx-n0".to_owned(),
                STORE_STRING,
                string(STSWX, 5, RA, RB),
                &[(RA, addr(0)), (RB, 0)],
                0,
            ));
            out
        }
        other => panic!("{other} is not a multiple or string unit"),
    };
    vectors.extend(
        INVALID_STRING_PROBES
            .iter()
            .filter(|(u, _)| *u == unit)
            .map(|(_, probe)| ended_case_vector(probe)),
    );
    if unit == "lswx" {
        // [PPC-Book1 p:48 s:3.3.6] A zero lswx count leaves RT undefined;
        // the console's case settles it.
        let converted = convert(&["ppu_reference_vectors".to_owned()]);
        vectors.extend(
            converted
                .candidates
                .get("lswx")
                .cloned()
                .unwrap_or_default(),
        );
    }
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit: PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors,
    }
}
