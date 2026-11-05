//! The PPU reference sets of the cache, synchronization and reservation
//! instructions, the three instruction groups and the six facilities.
//!
//! Each vector's expected end follows from the rule its citation names,
//! through the small model below, or carries the console capture that
//! answers it. A cache or barrier instruction changes no register and no
//! data byte. A load and reserve holds the 128-byte line of its address. A
//! store conditional writes only while the unit holds that line, and drops
//! the line either way. A word of a group and an unassigned word stop the
//! reference on a fault. A console run that ended the process gives the
//! same shape.

use super::capture::{
    capture_source, unobserved_stop, unsupported, value, ENTRY_VSCR, MEMORY_BASE,
};
use super::multiple::{ended_case_vector, ended_vector, fault_only_vector, model_vector_over};
use crate::ppu_reference::{
    PpuReferenceFacility, PpuReferenceInputState, PpuReferenceObservation, PpuReferenceProvenance,
    PpuReferenceSet, PpuReferenceState, PpuReferenceStateBase, PpuReferenceUnit,
    PpuReferenceVector, ReferenceField, PPU_REFERENCE_SET_SCHEMA_VERSION,
};
use cellgov_testkit::console_cases::PROFILE;

/// [PPC-Book2 p:18 s:3.2.1] `icbi` invalidates the instruction cache
/// block; it changes no register and no storage byte.
const ICBI: &str = "PPC-Book2 p:18 s:3.2.1";

/// [PPC-Book2 p:19 s:3.2.2] `dcbt` and `dcbtst` are hints; they change no
/// register and no storage byte.
const TOUCH: &str = "PPC-Book2 p:19 s:3.2.2";

/// [PPC-Book2 p:20 s:3.2.2] `dcbz` sets every byte of the block that
/// holds its address to zero.
const DCBZ: &str = "PPC-Book2 p:20 s:3.2.2";

/// [PPC-Book2 p:21 s:3.2.2] `dcbst` and `dcbf` write a modified block back
/// to storage; the storage bytes keep their values.
const FLUSH: &str = "PPC-Book2 p:21 s:3.2.2";

/// [PPC-Book2 p:22 s:3.3.1] `isync` waits for the preceding instructions;
/// it changes no register.
const ISYNC: &str = "PPC-Book2 p:22 s:3.3.1";

/// [PPC-Book2 p:24 s:3.3.2] `lwarx` and `ldarx` load the word or
/// doubleword at `(RA|0) + (RB)` and create a reservation for it.
const LOAD_RESERVE: &str = "PPC-Book2 p:24 s:3.3.2";

/// [PPC-Book2 p:25 s:3.3.2] `stwcx.` and `stdcx.` store when a
/// reservation exists for their address and not when none exists, set
/// CR0 to `0b00 || n || SO` with `n` 1 when they store, and clear the
/// reservation. Under a reservation for another location the architecture
/// leaves the store and `n` undefined.
const STORE_CONDITIONAL: &str = "PPC-Book2 p:25 s:3.3.2";

/// Why a documented vector leaves out what a store conditional under a
/// reservation for another location did.
const OTHER_LOCATION: &str = "under a reservation for another location the architecture leaves the store and CR0 EQ undefined; the console vectors record what the PPE does";

/// [PPC-Book2 p:26 s:3.3.3] `sync` orders storage accesses; it changes no
/// register.
const SYNC: &str = "PPC-Book2 p:26 s:3.3.3";

/// [PPC-Book2 p:28 s:3.3.3] `eieio` orders storage accesses; it changes
/// no register.
const EIEIO: &str = "PPC-Book2 p:28 s:3.3.3";

/// [PPC-Book3 p:71 s:5.5.9] A program interrupt follows an attempt to run
/// an illegal instruction, an optional instruction the implementation
/// does not provide, or a privileged instruction in problem state.
const PROGRAM_INTERRUPT: &str = "PPC-Book3 p:71 s:5.5.9";

/// [PPC-Book1 p:187 s:G] Primary opcodes 1, 4, 5, 6, 57, 60 and 61 are
/// illegal, as are the unused extended opcodes of 19, 30, 31, 56, 58,
/// 59, 62 and 63, and the word of all zeros.
const ILLEGAL: &str = "PPC-Book1 p:187 s:G";

/// [PPC-Book1 p:17 s:2.2] Instructions appear to run one at a time in
/// program order, so a load reads what an earlier store wrote.
const PROGRAM_ORDER: &str = "PPC-Book1 p:17 s:2.2";

/// [PPC-Book1 p:48 s:3.3.6] The string instructions move their bytes one
/// address after another, across a page as anywhere else.
const STRING: &str = "PPC-Book1 p:48 s:3.3.6";

/// The instruction units this module fills.
pub(super) const SYSTEM_INSTRUCTIONS: [&str; 13] = [
    "dcbf", "dcbst", "dcbt", "dcbtst", "dcbz", "eieio", "icbi", "isync", "sync", "lwarx", "ldarx",
    "stwcx.", "stdcx.",
];

/// The block a `dcbz` zeroes and a reservation covers.
///
/// [CBE-Handbook p:147 s:6.1.6.4] The PPE dcbz line is 128 bytes.
/// [CBE-Handbook p:578 s:20.2] The PPE reservation granule is 128 bytes.
const BLOCK: u64 = 128;

/// The unit the reference runs.
const UNIT: u64 = 0;

fn x(xo: u32, rt: usize, ra: usize, rb: usize) -> u32 {
    (31 << 26) | ((rt as u32) << 21) | ((ra as u32) << 16) | ((rb as u32) << 11) | (xo << 1)
}

fn d(op: u32, rt: usize, ra: usize, disp: i16) -> u32 {
    (op << 26) | ((rt as u32) << 21) | ((ra as u32) << 16) | u32::from(disp as u16)
}

const DCBST: u32 = 54;
const DCBF: u32 = 86;
const DCBTST: u32 = 246;
const DCBT: u32 = 278;
const ICBI_XO: u32 = 982;
const DCBZ_XO: u32 = 1014;
const LWARX: u32 = 20;
const LDARX: u32 = 84;
const STWCX: u32 = 150;
const STDCX: u32 = 214;

fn stwcx(rs: usize, ra: usize, rb: usize) -> u32 {
    x(STWCX, rs, ra, rb) | 1
}

fn stdcx(rs: usize, ra: usize, rb: usize) -> u32 {
    x(STDCX, rs, ra, rb) | 1
}

/// `ori r0, r0, 0`.
const NOP: u32 = 24 << 26;

/// The address `off` bytes into the data.
fn addr(off: u64) -> u64 {
    MEMORY_BASE + off
}

/// `len` data bytes: no two adjacent bytes equal, and both values of each
/// byte's top bit.
fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(0x3b).wrapping_add(0x91))
        .collect()
}

/// The registers and data a model step reads and writes.
#[derive(Debug, Clone)]
struct Machine {
    gpr: [u64; 32],
    cr: u32,
    xer: u64,
    data: Vec<u8>,
    reservation: Option<u64>,
}

impl Machine {
    /// A start with a distinct value per GPR, `regs` over them, and `data`.
    fn new(regs: &[(usize, u64)], data: Vec<u8>) -> Self {
        let mut gpr: [u64; 32] =
            std::array::from_fn(|n| 0x6c6c_0000_3939_0000 | ((n as u64) << 8) | n as u64);
        for &(n, v) in regs {
            gpr[n] = v;
        }
        Self {
            gpr,
            cr: 0x4567_89ab,
            xer: 0,
            data,
            reservation: None,
        }
    }

    fn at(&self, ea: u64, len: u64) -> usize {
        let at = ea.wrapping_sub(MEMORY_BASE);
        assert!(
            at + len <= self.data.len() as u64,
            "an access at {ea:#x} leaves the data"
        );
        at as usize
    }

    fn base(&self, ra: usize) -> u64 {
        if ra == 0 {
            0
        } else {
            self.gpr[ra]
        }
    }

    fn load(&self, ea: u64, len: u64) -> u64 {
        let at = self.at(ea, len);
        self.data[at..at + len as usize]
            .iter()
            .fold(0, |v, &b| (v << 8) | u64::from(b))
    }

    fn store(&mut self, ea: u64, len: u64, v: u64) {
        let at = self.at(ea, len);
        let bytes = v.to_be_bytes();
        self.data[at..at + len as usize].copy_from_slice(&bytes[8 - len as usize..]);
    }

    /// Runs one word by the cited rules.
    fn step(&mut self, word: u32) {
        let rt = ((word >> 21) & 31) as usize;
        let ra = ((word >> 16) & 31) as usize;
        let rb = ((word >> 11) & 31) as usize;
        let disp = i64::from(word as u16 as i16) as u64;
        let indexed = self.base(ra).wrapping_add(self.gpr[rb]);
        let displaced = self.base(ra).wrapping_add(disp);
        match (word >> 26, (word >> 1) & 0x3ff) {
            (24, _) | (19, 150) => {}
            (31, DCBST | DCBF | DCBTST | DCBT | ICBI_XO | 598 | 854) => {}
            (31, DCBZ_XO) => {
                let block = indexed & !(BLOCK - 1);
                for k in 0..BLOCK {
                    self.store(block + k, 1, 0);
                }
            }
            (31, LWARX) => self.load_reserve(rt, indexed, 4),
            (31, LDARX) => self.load_reserve(rt, indexed, 8),
            (31, STWCX) => self.store_conditional(rt, indexed, 4),
            (31, STDCX) => self.store_conditional(rt, indexed, 8),
            (32, _) => self.gpr[rt] = self.load(displaced, 4),
            (34, _) => self.gpr[rt] = self.load(displaced, 1),
            (36, _) => self.store(displaced, 4, self.gpr[rt]),
            (38, _) => self.store(displaced, 1, self.gpr[rt]),
            (44, _) => self.store(displaced, 2, self.gpr[rt]),
            (58, _) if word & 3 == 0 => self.gpr[rt] = self.load(displaced, 8),
            (62, _) if word & 3 == 0 => self.store(displaced, 8, self.gpr[rt]),
            _ => panic!("{word:#010x} is not a word the model runs"),
        }
    }

    fn load_reserve(&mut self, rt: usize, ea: u64, len: u64) {
        self.gpr[rt] = self.load(ea, len);
        self.reservation = Some(ea & !(BLOCK - 1));
    }

    fn store_conditional(&mut self, rs: usize, ea: u64, len: u64) {
        let held = self.reservation == Some(ea & !(BLOCK - 1));
        if held {
            self.store(ea, len, self.gpr[rs]);
        }
        let so = ((self.xer >> 31) & 1) as u32;
        self.cr = (self.cr & 0x0fff_ffff) | (u32::from(held) << 29) | (so << 28);
        self.reservation = None;
    }
}

fn input(m: &Machine) -> PpuReferenceInputState {
    PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: (0..32u8).map(|n| (n, m.gpr[usize::from(n)])).collect(),
        fpr: Default::default(),
        vr_hex: Default::default(),
        pc: 0,
        cr: m.cr,
        lr: 0,
        ctr: 0,
        xer: m.xer,
        fpscr: Some(0),
        vrsave: 0,
        vscr: Some(ENTRY_VSCR),
        tb: 0,
        reservation: m.reservation,
    }
}

const RUNTIME: &str = "the vector does not represent the runtime's effect pipeline";

/// The expected end of `words` run to completion from a start, as `m`.
fn expected(m: &Machine, words: usize) -> PpuReferenceObservation {
    PpuReferenceObservation {
        state: PpuReferenceState {
            gpr: value(m.gpr.to_vec()),
            fpr: value(vec![0; 32]),
            vr_hex: value(vec!["0".repeat(32); 32]),
            pc: value(4 * words as u64),
            cr: value(m.cr),
            lr: value(0),
            ctr: value(0),
            xer: value(m.xer),
            fpscr: value(0),
            vrsave: value(0),
            vscr: value(ENTRY_VSCR),
            tb: ReferenceField::Undefined {
                reason: "the time base advances while the vector runs".to_owned(),
            },
            reservation: value(m.reservation),
        },
        memory: value(m.data.clone()),
        stop: unobserved_stop(),
        retired: value(words as u64),
        staged_effects: unsupported(RUNTIME),
        committed_effects: unsupported(RUNTIME),
        reservations: value(m.reservation.map(|line| (UNIT, line)).into_iter().collect()),
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

/// A vector of `words` from `start`, its end by the model.
fn model_vector(
    name: String,
    provenance: PpuReferenceProvenance,
    words: Vec<u32>,
    start: Machine,
) -> PpuReferenceVector {
    let mut end = start.clone();
    for &w in &words {
        end.step(w);
    }
    PpuReferenceVector {
        name,
        provenance,
        initial_state: input(&start),
        memory_base: MEMORY_BASE,
        initial_memory: start.data,
        expected: expected(&end, words.len()),
        words,
    }
}

/// A cache block instruction over the data: at a line's start, inside a
/// line, at the next line, and with RA and RB both naming registers.
fn cache_vectors(unit: &str, xo: u32, citation: &str) -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = [0, 5, 128]
        .into_iter()
        .map(|off| {
            model_vector(
                format!("{unit}-ea{off}"),
                documented(citation, &format!("{unit}-ea{off}")),
                vec![x(xo, 0, 0, 9)],
                Machine::new(&[(9, addr(off))], pattern(256)),
            )
        })
        .collect();
    out.push(model_vector(
        format!("{unit}-ra-rb"),
        documented(citation, &format!("{unit}-ra-rb")),
        vec![x(xo, 0, 10, 9)],
        Machine::new(&[(10, addr(64)), (9, 100)], pattern(256)),
    ));
    out
}

/// The `ppu_cache_block` capture: `dcbst` on its first line, `dcbf` on its
/// second and `icbi` on its first leave the lines as they were, and each
/// touch hint on unmapped page 0 completes.
fn cache_capture(unit: &str, xo: u32) -> PpuReferenceVector {
    let ea = match unit {
        "dcbf" => addr(BLOCK),
        "dcbt" | "dcbtst" => 0x100,
        _ => addr(0),
    };
    model_vector(
        format!("{unit}-console"),
        captured("ppu_cache_block"),
        vec![x(xo, 0, 0, 9)],
        Machine::new(&[(9, ea)], pattern(256)),
    )
}

/// A barrier word from a start with CR, XER and every GPR set.
fn barrier_vector(name: &str, citation: &str, word: u32) -> PpuReferenceVector {
    let mut start = Machine::new(&[], pattern(16));
    start.xer = 0xa000_0000;
    model_vector(
        name.to_owned(),
        documented(citation, name),
        vec![word],
        start,
    )
}

fn sync_vectors() -> Vec<PpuReferenceVector> {
    let mut out = vec![
        barrier_vector("sync-l0", SYNC, x(598, 0, 0, 0)),
        barrier_vector("lwsync-l1", SYNC, x(598, 1, 0, 0)),
    ];
    // The console capture runs `sync 3`, an L the document reserves, and
    // the run completes with r3 as it was.
    let mut start = Machine::new(&[(3, 0x1234)], pattern(16));
    start.cr = 0;
    out.push(model_vector(
        "sync-l3-console".to_owned(),
        captured("ppu_may_trap_sync_l3"),
        vec![x(598, 3, 0, 0)],
        start,
    ));
    out
}

/// `dcbz` over 512 bytes of 0xee, then the console's capture.
///
/// The documented vectors run `dcbz`:
///
/// - at the first or the last byte of each of the four blocks;
/// - with RA and RB both naming registers.
fn dcbz_vectors(unit_name: &str) -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = [0, 255, 256, 511]
        .into_iter()
        .map(|off| {
            let name = format!("{unit_name}-ea{off}");
            model_vector(
                name.clone(),
                documented(DCBZ, &name),
                vec![x(DCBZ_XO, 0, 0, 9)],
                Machine::new(&[(9, addr(off))], vec![0xee; 512]),
            )
        })
        .collect();
    let name = format!("{unit_name}-ra-rb");
    out.push(model_vector(
        name.clone(),
        documented(DCBZ, &name),
        vec![x(DCBZ_XO, 0, 10, 9)],
        Machine::new(&[(10, addr(256)), (9, 100)], vec![0xee; 512]),
    ));
    // The ppu_granule_nan capture zeroes the 128 bytes from 168's block
    // start in a 512-byte buffer of 0xee.
    out.push(model_vector(
        format!("{unit_name}-ea168-console"),
        captured("ppu_granule_nan"),
        vec![x(DCBZ_XO, 0, 0, 9)],
        Machine::new(&[(9, addr(168))], vec![0xee; 512]),
    ));
    out
}

/// The load and reserve vectors by the rules, then the misaligned console
/// capture, which ended the child thread.
///
/// The documented vectors run the word:
///
/// - at the first, second and last element of the first line;
/// - at the first and last element of the next line;
/// - in the RA = 0 form;
/// - over a reservation the unit already holds for another line.
fn load_reserve_vectors(unit: &str, xo: u32, len: u64) -> Vec<PpuReferenceVector> {
    let offsets = [0, len, BLOCK - len, BLOCK, 256 - len];
    let mut out: Vec<PpuReferenceVector> = offsets
        .into_iter()
        .map(|off| {
            let name = format!("{unit}-off{off}");
            model_vector(
                name.clone(),
                documented(LOAD_RESERVE, &name),
                vec![x(xo, 3, 10, 4)],
                Machine::new(&[(10, addr(0)), (4, off)], pattern(256)),
            )
        })
        .collect();
    let name = format!("{unit}-ra0");
    out.push(model_vector(
        name.clone(),
        documented(LOAD_RESERVE, &name),
        vec![x(xo, 3, 0, 9)],
        Machine::new(&[(9, addr(64))], pattern(256)),
    ));
    let name = format!("{unit}-replaces-a-held-line");
    let mut start = Machine::new(&[(10, addr(0)), (4, 0)], pattern(256));
    start.reservation = Some(addr(BLOCK));
    out.push(model_vector(
        name.clone(),
        documented(LOAD_RESERVE, &name),
        vec![x(xo, 3, 10, 4)],
        start,
    ));
    // The ppu_exception_seen probes run the word with r9 at 2 or 4 bytes
    // past an aligned address; the console removed the thread.
    let (probe, off) = if len == 4 {
        ("ppu_exception_seen_lwarx", 2)
    } else {
        ("ppu_exception_seen_ldarx", 4)
    };
    out.push(ended_vector(
        format!("{unit}-misaligned-console"),
        probe,
        vec![x(xo, 0, 0, 9)],
        &[(9, addr(off))],
        0,
    ));
    out
}

/// The store conditional vectors by the rules, then the console's pairs.
///
/// The documented vectors run the store:
///
/// - under a reservation for its line, at the line's start and end;
/// - under a reservation for another line;
/// - with no reservation;
/// - with SO set.
fn store_conditional_vectors(
    unit: &str,
    store: fn(usize, usize, usize) -> u32,
    len: u64,
) -> Vec<PpuReferenceVector> {
    let mut out = Vec::new();
    for (tag, off, held, xer) in [
        ("held-off0", 0, Some(0), 0),
        ("held-last", BLOCK - len, Some(0), 0),
        ("other-line", BLOCK, Some(0), 0),
        ("none", 0, None, 0),
        ("held-so", 0, Some(0), 0x8000_0000),
    ] {
        let name = format!("{unit}-{tag}");
        let mut start = Machine::new(&[(10, addr(0)), (4, off)], pattern(256));
        start.reservation = held.map(addr);
        start.xer = xer;
        let mut v = model_vector(
            name.clone(),
            documented(STORE_CONDITIONAL, &name),
            vec![store(3, 10, 4)],
            start,
        );
        if tag == "other-line" {
            v.expected.state.cr = ReferenceField::Undefined {
                reason: OTHER_LOCATION.to_owned(),
            };
            v.expected.memory = ReferenceField::Undefined {
                reason: OTHER_LOCATION.to_owned(),
            };
        }
        out.push(v);
    }
    out.extend(edge_pairs(unit, store, len));
    out
}

/// The `ppu_reservation_edges` pairs for a store conditional: a load and
/// reserve at a line's start, then the store at each offset the probe
/// sends. The console stored at every offset inside the line and at none
/// in the next. It also stored after a load and reserve of the other
/// width, and after a `dcbf` or `dcbst` of the line.
fn edge_pairs(
    unit: &str,
    store: fn(usize, usize, usize) -> u32,
    len: u64,
) -> Vec<PpuReferenceVector> {
    let (same, other) = if len == 4 {
        (LWARX, LDARX)
    } else {
        (LDARX, LWARX)
    };
    let offsets = [0, len, 64, BLOCK - len, BLOCK];
    let mut out: Vec<PpuReferenceVector> = offsets
        .into_iter()
        .map(|off| {
            model_vector(
                format!("{unit}-after-reserve-off{off}-console"),
                captured("ppu_reservation_edges"),
                vec![x(same, 5, 10, 0), store(3, 10, 6)],
                Machine::new(&[(0, 0), (10, addr(0)), (6, off)], pattern(256)),
            )
        })
        .collect();
    out.push(model_vector(
        format!("{unit}-after-other-width-console"),
        captured("ppu_reservation_edges"),
        vec![x(other, 5, 10, 0), store(3, 10, 6)],
        Machine::new(&[(0, 0), (10, addr(0)), (6, 0)], pattern(256)),
    ));
    if len == 4 {
        for (name, xo) in [("dcbf", DCBF), ("dcbst", DCBST)] {
            out.push(model_vector(
                format!("{unit}-after-{name}-console"),
                captured("ppu_reservation_edges"),
                vec![x(LWARX, 5, 10, 0), x(xo, 0, 0, 10), store(3, 10, 6)],
                Machine::new(&[(0, 0), (10, addr(0)), (6, 0)], pattern(256)),
            ));
        }
    }
    out
}

/// A documented vector of `word` that expects the reference to stop on a
/// fault.
fn fault_vector(name: &str, citation: &str, word: u32) -> PpuReferenceVector {
    let reason =
        "the interrupt hands the instruction to the system; the vector checks the fault stop alone";
    fault_only_vector(
        name.to_owned(),
        documented(citation, name),
        reason,
        vec![word],
        &[],
        0,
    )
}

fn xl(xo: u32) -> u32 {
    (19 << 26) | (xo << 1)
}

/// The words the PPE leaves unimplemented: the console captures, then one
/// documented vector for each of the others.
fn unimplemented_vectors() -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = [
        "ppu_may_trap_popcntb",
        "ppu_may_trap_mcrxr",
        "ppu_may_trap_mfspr280",
        "ppu_may_trap_lq_rt_ra",
    ]
    .into_iter()
    .map(ended_case_vector)
    .collect();
    let words = [
        ("fre", (63 << 26) | (1 << 21) | (2 << 11) | (24 << 1)),
        ("frsqrtes", (59 << 26) | (1 << 21) | (2 << 11) | (26 << 1)),
        ("eciwx", x(310, 3, 4, 5)),
        ("ecowx", x(438, 3, 4, 5)),
        ("tlbia", x(370, 0, 0, 0)),
        ("mtsr", x(210, 3, 2, 0)),
        ("mtsrin", x(242, 3, 0, 4)),
        ("mfsr", x(595, 3, 2, 0)),
        ("mfsrin", x(659, 3, 0, 4)),
    ];
    out.extend(
        words
            .into_iter()
            .map(|(name, word)| fault_vector(name, PROGRAM_INTERRUPT, word)),
    );
    out
}

/// The privileged words a problem-state program may not run: the console
/// captures, then one documented vector for each of the others.
fn privileged_vectors() -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = [
        "ppu_may_trap_mfmsr",
        "ppu_may_trap_slbmfee",
        "ppu_may_trap_slbmfev",
        "ppu_may_trap_lq",
        "ppu_may_trap_stq",
        "ppu_may_trap_mfspr152",
        "ppu_may_trap_mfspr272",
        "ppu_may_trap_mfspr284",
        "ppu_may_trap_mfspr287",
        "ppu_may_trap_mfspr1022",
    ]
    .into_iter()
    .map(ended_case_vector)
    .collect();
    // The probe clears r3 and runs mtmsrd r3; the console removed the
    // thread.
    out.push(ended_vector(
        "mtmsrd-console".to_owned(),
        "ppu_exception_seen_mtmsrd",
        vec![0x7c60_0164],
        &[(3, 0)],
        0,
    ));
    let words = [
        ("rfid", xl(18)),
        ("hrfid", xl(274)),
        ("mtmsr", x(146, 3, 0, 0)),
        ("slbia", x(498, 0, 0, 0)),
        ("slbie", x(434, 0, 0, 4)),
        ("slbmte", x(402, 3, 0, 4)),
        ("tlbie", x(306, 0, 0, 4)),
        ("tlbiel", x(274, 0, 0, 4)),
        ("tlbsync", x(566, 0, 0, 0)),
    ];
    out.extend(
        words
            .into_iter()
            .map(|(name, word)| fault_vector(name, PROGRAM_INTERRUPT, word)),
    );
    out
}

/// The illegal primary opcodes the PPE leaves unassigned. Opcode 4 is
/// absent because it holds the PPE's vector instructions.
const ILLEGAL_PRIMARIES: [u32; 6] = [1, 5, 6, 57, 60, 61];

/// Extended opcodes Book I's opcode maps leave unused, eight under each
/// of 19, 31, 59 and 63, spread across the field. None is an opcode the
/// PPE adds, such as its vector loads and stores or its byte-reverse
/// doubleword forms.
const UNUSED_EXTENDED: [(u32, [u32; 8]); 4] = [
    (19, [1, 67, 131, 196, 261, 326, 389, 454]),
    (31, [1, 82, 152, 213, 277, 337, 400, 463]),
    (59, [0, 64, 128, 192, 256, 320, 384, 448]),
    (63, [1, 71, 135, 198, 260, 323, 385, 443]),
];

fn unassigned_vectors() -> Vec<PpuReferenceVector> {
    let mut out = vec![fault_vector("all-zero", ILLEGAL, 0)];
    for p in ILLEGAL_PRIMARIES {
        out.push(fault_vector(
            &format!("primary-{p}"),
            ILLEGAL,
            (p << 26) | (3 << 21) | (4 << 16) | 0x1234,
        ));
    }
    for (p, xos) in UNUSED_EXTENDED {
        for xo in xos {
            out.push(fault_vector(
                &format!("primary-{p}-xo-{xo}"),
                ILLEGAL,
                (p << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (xo << 1),
            ));
        }
    }
    // The ppu_exception_seen_illegal probe runs primary opcode 1 alone;
    // the console removed the thread.
    out.push(ended_vector(
        "primary-1-console".to_owned(),
        "ppu_exception_seen_illegal",
        vec![0x0400_0000],
        &[],
        0,
    ));
    out
}

/// The `ppu_entry_state` capture's two save areas: each thread's
/// registers at its first instruction, run through one no-op. Each
/// vector holds what a new thread starts with on the console.
fn thread_entry_vectors() -> Vec<PpuReferenceVector> {
    const SAVE: usize = 1088;
    let probe = "ppu_entry_state";
    let path = cellgov_testkit::console_cases::probe_dir(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &frame[8..];
    assert_eq!(p.len(), 16 + 2 * SAVE, "{probe}: payload size");
    ["main", "child"]
        .into_iter()
        .enumerate()
        .map(|(k, thread)| {
            let save = &p[16 + k * SAVE..16 + (k + 1) * SAVE];
            entry_vector(format!("thread-entry-{thread}"), captured(probe), save)
        })
        .collect()
}

/// One entry save area as a vector that runs a no-op.
fn entry_vector(
    name: String,
    provenance: PpuReferenceProvenance,
    save: &[u8],
) -> PpuReferenceVector {
    let dw = |at: usize| u64::from_be_bytes(save[at..at + 8].try_into().expect("8 bytes"));
    let qw = |at: usize| u128::from_be_bytes(save[at..at + 16].try_into().expect("16 bytes"));
    let gpr: [u64; 32] = std::array::from_fn(|n| dw(8 * n));
    let fpr: [u64; 32] = std::array::from_fn(|n| dw(304 + 8 * n));
    let vr: [u128; 32] = std::array::from_fn(|n| qw(560 + 16 * n));
    let (cr, xer, ctr, lr) = (dw(256) as u32, dw(264), dw(272), dw(280));
    let (vrsave, fpscr, vscr) = (dw(288) as u32, dw(296) as u32, qw(1072) as u32);
    let vr_hex: Vec<String> = vr.iter().map(|v| format!("{v:032x}")).collect();
    let initial_state = PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: (0..32u8).map(|n| (n, gpr[usize::from(n)])).collect(),
        fpr: (0..32u8).map(|n| (n, fpr[usize::from(n)])).collect(),
        vr_hex: (0..32u8)
            .map(|n| (n, vr_hex[usize::from(n)].clone()))
            .collect(),
        pc: 0,
        cr,
        lr,
        ctr,
        xer,
        fpscr: Some(fpscr),
        vrsave,
        vscr: Some(vscr),
        tb: 0,
        reservation: None,
    };
    let mut expected = expected(&Machine::new(&[], vec![0; 16]), 1);
    expected.state = PpuReferenceState {
        gpr: value(gpr.to_vec()),
        fpr: value(fpr.to_vec()),
        vr_hex: value(vr_hex),
        pc: value(4),
        cr: value(cr),
        lr: value(lr),
        ctr: value(ctr),
        xer: value(xer),
        fpscr: value(fpscr),
        vrsave: value(vrsave),
        vscr: value(vscr),
        tb: expected.state.tb,
        reservation: value(None),
    };
    PpuReferenceVector {
        name,
        provenance,
        words: vec![NOP],
        initial_state,
        memory_base: MEMORY_BASE,
        initial_memory: vec![0; 16],
        expected,
    }
}

/// The reservation facility: the granule edge the console found, a
/// reservation a successful store consumes, one a failed store drops,
/// and one a second load and reserve moves.
fn reservation_vectors() -> Vec<PpuReferenceVector> {
    let mut out: Vec<PpuReferenceVector> = [BLOCK - 4, BLOCK]
        .into_iter()
        .map(|off| {
            model_vector(
                format!("granule-off{off}-console"),
                captured("ppu_reservation_edges"),
                vec![x(LWARX, 5, 10, 0), stwcx(3, 10, 6)],
                Machine::new(&[(0, 0), (10, addr(0)), (6, off)], pattern(256)),
            )
        })
        .collect();
    out.push(model_vector(
        "consumed-by-a-store".to_owned(),
        documented(STORE_CONDITIONAL, "consumed-by-a-store"),
        vec![x(LWARX, 5, 10, 0), stwcx(3, 10, 0), stwcx(4, 10, 0)],
        Machine::new(&[(0, 0), (10, addr(0))], pattern(256)),
    ));
    // The first store misses the reserved line, so the architecture leaves
    // its write undefined. It clears the reservation either way, so the
    // second store fails and sets a defined CR0.
    let mut held = Machine::new(&[(0, 0), (10, addr(0)), (6, BLOCK)], pattern(256));
    held.reservation = Some(addr(0));
    let mut dropped = model_vector(
        "dropped-by-a-failed-store".to_owned(),
        documented(STORE_CONDITIONAL, "dropped-by-a-failed-store"),
        vec![stwcx(3, 10, 6), stwcx(4, 10, 0)],
        held,
    );
    dropped.expected.memory = ReferenceField::Undefined {
        reason: OTHER_LOCATION.to_owned(),
    };
    out.push(dropped);
    // A reservation for line 1, then a load and reserve of line 0: the
    // store to line 0 finds its reservation and stores.
    let mut elsewhere = Machine::new(&[(0, 0), (10, addr(0))], pattern(256));
    elsewhere.reservation = Some(addr(BLOCK));
    out.push(model_vector(
        "moved-by-a-second-reserve".to_owned(),
        documented(LOAD_RESERVE, "moved-by-a-second-reserve"),
        vec![x(LWARX, 5, 10, 0), stwcx(3, 10, 0)],
        elsewhere,
    ));
    out
}

/// One read of the time base and one of its upper word.
fn time_base_vectors() -> Vec<PpuReferenceVector> {
    let mftb =
        |tbr: u32| (31 << 26) | (3 << 21) | ((tbr & 31) << 16) | ((tbr >> 5) << 11) | (371 << 1);
    [("tb", 268), ("tbu", 269)]
        .into_iter()
        .map(|(name, tbr)| {
            let mut v = model_vector(
                format!("read-{name}"),
                captured("ppu_timebase_reads"),
                vec![NOP],
                Machine::new(&[], vec![0; 16]),
            );
            v.words = vec![mftb(tbr)];
            v.expected.state.gpr = ReferenceField::Undefined {
                reason: "r3 holds a time-base read, which depends on when it runs; the console's later reads are never below it".to_owned(),
            };
            v
        })
        .collect()
}

/// A store, then a load that overlaps the bytes it wrote, for five pairs
/// of store and load widths.
fn store_forwarding_vectors() -> Vec<PpuReferenceVector> {
    let cases = [
        ("word-word", d(36, 3, 10, 0), d(32, 4, 10, 0)),
        ("byte-word", d(38, 3, 10, 1), d(32, 4, 10, 0)),
        ("doubleword-word", d(62, 3, 10, 0), d(32, 4, 10, 4)),
        ("halfword-doubleword", d(44, 3, 10, 6), d(58, 4, 10, 0)),
        ("word-byte", d(36, 3, 10, 8), d(34, 4, 10, 11)),
    ];
    cases
        .into_iter()
        .map(|(name, store, load)| {
            model_vector(
                name.to_owned(),
                documented(PROGRAM_ORDER, name),
                vec![store, load],
                Machine::new(&[(10, addr(0)), (3, 0x0123_4567_89ab_cdef)], pattern(16)),
            )
        })
        .collect()
}

/// String loads and stores whose bytes run across the 4 KiB page edge of
/// the data.
fn string_across_page_vectors() -> Vec<PpuReferenceVector> {
    const PAGE: u64 = 0x1000;
    let s = |xo: u32, rt: usize, ra: usize, rb: usize| x(xo, rt, ra, rb);
    let runs = [
        ("lswi-n8", s(597, 5, 9, 8), PAGE - 4, 0),
        ("stswi-n8", s(725, 5, 9, 8), PAGE - 4, 0),
        ("lswx-n12", s(533, 5, 9, 4), PAGE - 6, 12),
        ("stswx-n12", s(661, 5, 9, 4), PAGE - 6, 12),
    ];
    runs.into_iter()
        .map(|(name, word, off, xer)| {
            model_vector_over(
                name.to_owned(),
                STRING,
                word,
                &[(9, addr(off)), (4, 0)],
                xer,
                pattern(PAGE as usize + 16),
            )
        })
        .collect()
}

fn set(unit: PpuReferenceUnit, vectors: Vec<PpuReferenceVector>) -> PpuReferenceSet {
    PpuReferenceSet {
        schema_version: PPU_REFERENCE_SET_SCHEMA_VERSION,
        unit,
        vectors,
    }
}

/// The set this module writes for an instruction unit.
pub(super) fn instruction_set(unit: &str) -> PpuReferenceSet {
    let vectors = match unit {
        "dcbf" => [
            cache_vectors(unit, DCBF, FLUSH),
            vec![cache_capture(unit, DCBF)],
        ]
        .concat(),
        "dcbst" => [
            cache_vectors(unit, DCBST, FLUSH),
            vec![cache_capture(unit, DCBST)],
        ]
        .concat(),
        "icbi" => [
            cache_vectors(unit, ICBI_XO, ICBI),
            vec![cache_capture(unit, ICBI_XO)],
        ]
        .concat(),
        "dcbt" => [
            cache_vectors(unit, DCBT, TOUCH),
            vec![cache_capture(unit, DCBT)],
        ]
        .concat(),
        "dcbtst" => [
            cache_vectors(unit, DCBTST, TOUCH),
            vec![cache_capture(unit, DCBTST)],
        ]
        .concat(),
        "dcbz" => dcbz_vectors(unit),
        "eieio" => vec![barrier_vector("eieio", EIEIO, x(854, 0, 0, 0))],
        "isync" => vec![barrier_vector("isync", ISYNC, xl(150))],
        "sync" => sync_vectors(),
        "lwarx" => load_reserve_vectors(unit, LWARX, 4),
        "ldarx" => load_reserve_vectors(unit, LDARX, 8),
        "stwcx." => store_conditional_vectors(unit, stwcx, 4),
        "stdcx." => store_conditional_vectors(unit, stdcx, 8),
        other => panic!("{other} is not a unit of this module"),
    };
    set(
        PpuReferenceUnit::Instruction {
            mnemonic: unit.to_owned(),
        },
        vectors,
    )
}

/// The group, unassigned and facility sets, each with its file name.
pub(super) fn other_sets() -> Vec<(&'static str, PpuReferenceSet)> {
    use PpuReferenceFacility as F;
    let facility = |name| PpuReferenceUnit::Facility { name };
    vec![
        (
            "ppe_unimplemented.json",
            set(PpuReferenceUnit::PpeUnimplemented, unimplemented_vectors()),
        ),
        (
            "privileged.json",
            set(PpuReferenceUnit::Privileged, privileged_vectors()),
        ),
        (
            "unassigned_opcodes.json",
            set(PpuReferenceUnit::UnassignedOpcodes, unassigned_vectors()),
        ),
        (
            "facility_thread_entry.json",
            set(facility(F::ThreadEntry), thread_entry_vectors()),
        ),
        (
            "facility_reservation.json",
            set(facility(F::Reservation), reservation_vectors()),
        ),
        (
            "facility_dcbz_width.json",
            set(facility(F::DcbzWidth), dcbz_vectors("dcbz-width")),
        ),
        (
            "facility_time_base.json",
            set(facility(F::TimeBase), time_base_vectors()),
        ),
        (
            "facility_store_forwarding.json",
            set(facility(F::StoreForwarding), store_forwarding_vectors()),
        ),
        (
            "facility_string_across_page.json",
            set(facility(F::StringAcrossPage), string_across_page_vectors()),
        ),
    ]
}
