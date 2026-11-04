//! The table-driven PPU console probes: the case table, the generator
//! that renders a probe's `ppu/cases.h` from it, the capture reader and
//! the replay through the differential harness.
//!
//! A probe's `tests/micro/<probe>/cases.tsv` is read by both sides. The
//! probe runs each case twice and records the registers the row names,
//! XER, CR and FPSCR (`tests/micro/common/cgov_cases.h` gives the frame
//! layout). The replay runs the same words through `execute` from the
//! same registers and compares each recorded component bit for bit.
//!
//! [PPC-Book1 p:3 s:1.5.1] An undefined value may vary between
//! executions, so the two console runs are compared with each other as
//! well as with CellGov.

use std::path::{Path, PathBuf};

use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::{
    execute_state, run_case, CaseOutcome, InstructionCase, MemorySnapshot, OracleSource,
    PpuStateSnapshot,
};
use cellgov_ppu::instruction::AsmText;

/// The reference console profile every capture here is read from.
pub const PROFILE: &str = "cech20-cex-493";

/// The table-driven probes, each with its own `cases.tsv`.
pub const PROBES: [&str; 7] = [
    "ppu_int_edges",
    "ppu_may_trap_mcrxr",
    "ppu_may_trap_popcntb",
    "ppu_may_trap_mfspr136",
    "ppu_may_trap_mfspr259",
    "ppu_may_trap_mfspr268",
    "ppu_may_trap_mfspr269",
];

/// The GPRs a case may load or record; r0, r11 and r12 are the
/// routine's own.
const GPRS: std::ops::RangeInclusive<u8> = 3..=10;

/// The FPRs a case may load or record; f0 is the routine's own.
const FPRS: std::ops::RangeInclusive<u8> = 1..=13;

/// The most registers one case records.
const SLOTS: usize = 4;

/// Bytes one run of one case takes in the frame.
const RUN_BYTES: usize = 64;

/// The frame's header bytes.
const HEADER_BYTES: usize = 16;

/// Word 0 of a frame whose cases all ran.
const DONE: u32 = 0x4341_5345;

/// Word 0 of a progress record.
const PROGRESS: u32 = 0x5052_4f47;

/// A register a case loads or records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    /// GPR `n`.
    Gpr(u8),
    /// FPR `n`.
    Fpr(u8),
}

impl Reg {
    fn parse(text: &str) -> Result<Self, String> {
        let (kind, n) = text.split_at(1);
        let n: u8 = n.parse().map_err(|_| format!("register {text:?}"))?;
        match kind {
            "r" if GPRS.contains(&n) => Ok(Reg::Gpr(n)),
            "f" if FPRS.contains(&n) => Ok(Reg::Fpr(n)),
            _ => Err(format!("register {text:?} is not one a case may use")),
        }
    }

    fn write(self, s: &mut PpuStateSnapshot, v: u64) {
        match self {
            Reg::Gpr(n) => s.gpr[usize::from(n)] = v,
            Reg::Fpr(n) => s.fpr[usize::from(n)] = v,
        }
    }

    fn name(self) -> String {
        match self {
            Reg::Gpr(n) => format!("r{n}"),
            Reg::Fpr(n) => format!("f{n}"),
        }
    }
}

/// One row of a case table.
#[derive(Debug, Clone)]
pub struct Case {
    /// Stable id.
    pub id: String,
    /// The words as assembly text, `;` between words.
    pub asm: String,
    /// The instruction words run in order.
    pub words: Vec<u32>,
    /// Registers loaded before the words, in table order.
    pub inputs: Vec<(Reg, u64)>,
    /// CR before the words.
    pub cr: u32,
    /// XER before the words.
    pub xer: u64,
    /// FPSCR before the words.
    pub fpscr: u32,
    /// The registers recorded after the words, at most [`SLOTS`].
    pub record: Vec<Reg>,
    /// Whether the case may end the process.
    pub may_end_process: bool,
}

impl Case {
    /// The value a recorded register holds before the words when no
    /// input names it: distinct per case and register, so an unchanged
    /// register shows.
    fn preset(&self, index: usize, reg: Reg) -> u64 {
        let tag = match reg {
            Reg::Gpr(n) => u64::from(n),
            Reg::Fpr(n) => 0x80 | u64::from(n),
        };
        0x5a5a_0000_0000_0000 | ((index as u64) << 8) | tag
    }

    /// Every register loaded before the words: the inputs, then a preset
    /// for each recorded register no input names.
    fn loads(&self, index: usize) -> Vec<(Reg, u64)> {
        let mut out = self.inputs.clone();
        for &r in &self.record {
            if !out.iter().any(|&(i, _)| i == r) {
                out.push((r, self.preset(index, r)));
            }
        }
        out
    }

    /// The state the words start from.
    pub fn initial(&self, index: usize) -> PpuStateSnapshot {
        let mut s = PpuStateSnapshot::zero();
        for (r, v) in self.loads(index) {
            r.write(&mut s, v);
        }
        s.cr = self.cr;
        s.xer = self.xer;
        s.fpscr = self.fpscr;
        s
    }
}

fn hex_u64(text: &str) -> Result<u64, String> {
    let digits = text
        .strip_prefix("0x")
        .ok_or_else(|| format!("{text:?} is not 0x-prefixed hex"))?;
    u64::from_str_radix(&digits.replace('_', ""), 16).map_err(|e| format!("{text:?}: {e}"))
}

fn list(field: &str) -> impl Iterator<Item = &str> {
    field.split(',').filter(|s| !s.is_empty() && *s != "-")
}

/// Parse a case table.
pub fn parse_table(text: &str) -> Result<Vec<Case>, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty table")?;
    if header != "id\tasm\twords\tinputs\trecord\tmay_end_process" {
        return Err(format!("header {header:?}"));
    }
    let mut cases: Vec<Case> = Vec::new();
    for (n, line) in lines.enumerate() {
        let at = |e: String| format!("row {}: {e}", n + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        let [id, asm, words, inputs, record, fatal] = fields[..] else {
            return Err(at(format!("{} fields", fields.len())));
        };
        let words = list(words)
            .map(|w| hex_u64(w).and_then(|v| u32::try_from(v).map_err(|e| e.to_string())))
            .collect::<Result<Vec<u32>, String>>()
            .map_err(at)?;
        if words.is_empty() {
            return Err(at("no words".into()));
        }
        let mut case = Case {
            id: id.to_string(),
            asm: asm.to_string(),
            words,
            inputs: Vec::new(),
            cr: 0,
            xer: 0,
            fpscr: 0,
            record: Vec::new(),
            may_end_process: match fatal {
                "0" => false,
                "1" => true,
                other => return Err(at(format!("may_end_process {other:?}"))),
            },
        };
        for item in list(inputs) {
            let (key, value) = item
                .split_once('=')
                .ok_or_else(|| at(format!("input {item:?}")))?;
            let value = hex_u64(value).map_err(at)?;
            let narrow = |v: u64| u32::try_from(v).map_err(|_| at(format!("{key} {v:#x}")));
            match key {
                "cr" => case.cr = narrow(value)?,
                "xer" => case.xer = value,
                "fpscr" => case.fpscr = narrow(value)?,
                reg => {
                    let reg = Reg::parse(reg).map_err(at)?;
                    if case.inputs.iter().any(|&(r, _)| r == reg) {
                        return Err(at(format!("{reg:?} loaded twice")));
                    }
                    case.inputs.push((reg, value));
                }
            }
        }
        for item in list(record) {
            let reg = Reg::parse(item).map_err(at)?;
            if case.record.contains(&reg) {
                return Err(at(format!("{reg:?} recorded twice")));
            }
            case.record.push(reg);
        }
        if case.record.len() > SLOTS {
            return Err(at(format!("{} recorded registers", case.record.len())));
        }
        if cases.iter().any(|c| c.id == case.id) {
            return Err(at(format!("duplicate id {:?}", case.id)));
        }
        cases.push(case);
    }
    if cases.iter().any(|c| c.may_end_process) && cases.len() != 1 {
        return Err("a case that may end the process must be its table's only case".into());
    }
    Ok(cases)
}

/// `tests/micro/<probe>/`.
pub fn probe_dir(probe: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
}

/// The probe's parsed case table.
pub fn table(probe: &str) -> Vec<Case> {
    let path = probe_dir(probe).join("cases.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse_table(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The probe's `ppu/cases.h`, rendered from its table.
pub fn render(probe: &str, cases: &[Case]) -> String {
    let mut data: Vec<String> = vec!["0".into()];
    let mut asm: Vec<String> = Vec::new();
    let line = |asm: &mut Vec<String>, text: String| asm.push(format!("        \"{text}\\n\\t\""));
    line(&mut asm, "mr 12,%0".into());
    line(&mut asm, "mr 11,%1".into());
    line(&mut asm, "mffs 0".into());
    line(&mut asm, "stfd 0,0(11)".into());
    line(&mut asm, "addi 11,11,8".into());
    for (index, case) in cases.iter().enumerate() {
        let loads = case.loads(index);
        let mut block = vec![
            format!("0x{:016x}ULL", u64::from(case.cr)),
            format!("0x{:016x}ULL", case.xer),
            format!("0x{:016x}ULL", u64::from(case.fpscr)),
        ];
        block.extend(loads.iter().map(|&(_, v)| format!("0x{v:016x}ULL")));
        asm.push(format!("        /* {index} {} */", case.id));
        for run in 0..2 {
            line(&mut asm, "ld 0,0(11)".into());
            line(&mut asm, "mtcrf 0xff,0".into());
            line(&mut asm, "ld 0,8(11)".into());
            line(&mut asm, "mtxer 0".into());
            line(&mut asm, "lfd 0,16(11)".into());
            line(&mut asm, "mtfsf 0xff,0".into());
            for (k, &(r, _)) in loads.iter().enumerate() {
                let off = 24 + 8 * k;
                match r {
                    Reg::Gpr(n) => line(&mut asm, format!("ld {n},{off}(11)")),
                    Reg::Fpr(n) => line(&mut asm, format!("lfd {n},{off}(11)")),
                }
            }
            for &w in &case.words {
                line(&mut asm, format!(".long 0x{w:08x}"));
            }
            let base = RUN_BYTES * run;
            line(&mut asm, "mfcr 0".into());
            line(&mut asm, format!("stw 0,{}(12)", base + 40));
            line(&mut asm, "mfxer 0".into());
            line(&mut asm, format!("std 0,{}(12)", base + 32));
            for (slot, &r) in case.record.iter().enumerate() {
                match r {
                    Reg::Gpr(n) => line(&mut asm, format!("std {n},{}(12)", base + 8 * slot)),
                    Reg::Fpr(n) => line(&mut asm, format!("stfd {n},{}(12)", base + 8 * slot)),
                }
            }
            line(&mut asm, "mffs 0".into());
            line(&mut asm, format!("stfd 0,{}(12)", base + 48));
        }
        line(&mut asm, format!("addi 11,11,{}", 8 * block.len()));
        line(&mut asm, format!("addi 12,12,{}", 2 * RUN_BYTES));
        data.extend(block);
    }
    line(&mut asm, "lfd 0,0(%1)".into());
    line(&mut asm, "mtfsf 0xff,0".into());
    let fatal = cases.iter().any(|c| c.may_end_process);
    let mut out = String::new();
    out.push_str(&format!(
        "/* Generated from tests/micro/{probe}/cases.tsv by the cellgov_ppu test\n \
         * console_cases (CELLGOV_REGENERATE=1); do not edit.\n \
         *\n \
         * cgov_case_data: an FPSCR save slot, then per case CR, XER, FPSCR and\n \
         * each register the case loads. cgov_run_cases runs every case twice\n \
         * into the frame layout tests/micro/common/cgov_cases.h gives.\n \
         */\n\n"
    ));
    out.push_str(&format!("#define CGOV_CASE_COUNT {}\n", cases.len()));
    out.push_str(&format!("#define CGOV_CASE_FATAL {}\n\n", u8::from(fatal)));
    out.push_str("static unsigned long long cgov_case_data[] __attribute__((aligned(16))) = {\n");
    for chunk in data.chunks(3) {
        out.push_str(&format!("    {},\n", chunk.join(", ")));
    }
    out.push_str("};\n\n");
    out.push_str(
        "static void cgov_run_cases(unsigned char *res, unsigned long long *data)\n{\n    __asm__ volatile(\n",
    );
    for l in &asm {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(
        "        :\n        : \"r\"(res), \"r\"(data)\n        : \"r0\", \"r3\", \"r4\", \"r5\", \"r6\", \"r7\", \"r8\", \"r9\", \"r10\", \"r11\", \"r12\",\n          \"fr0\", \"fr1\", \"fr2\", \"fr3\", \"fr4\", \"fr5\", \"fr6\", \"fr7\", \"fr8\", \"fr9\", \"fr10\",\n          \"fr11\", \"fr12\", \"fr13\", \"cr0\", \"cr1\", \"cr2\", \"cr3\", \"cr4\", \"cr5\", \"cr6\", \"cr7\",\n          \"xer\", \"ctr\", \"memory\");\n}\n",
    );
    out
}

/// One run of one case as the console recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// The recorded registers, in table order; unused slots hold 0.
    pub slots: [u64; SLOTS],
    /// XER.
    pub xer: u64,
    /// CR.
    pub cr: u32,
    /// The FPSCR word of the `mffs` result.
    pub fpscr: u32,
}

/// What a probe's committed capture holds.
#[derive(Debug)]
pub enum Capture {
    /// Every case ran: each case's two runs.
    Done(Vec<[Run; 2]>),
    /// The run ended inside the case the progress record names.
    Ended {
        /// The index of that case.
        case: usize,
    },
}

/// `<probe>/<profile>`, the capture id the differential report prints.
pub fn capture_id(probe: &str) -> &'static str {
    Box::leak(format!("{probe}/{PROFILE}").into_boxed_str())
}

/// Read the probe's committed capture.
pub fn capture(probe: &str, cases: usize) -> Capture {
    let path = probe_dir(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    let payload = &frame[8..];
    assert_eq!(
        payload.len(),
        HEADER_BYTES + 2 * RUN_BYTES * cases,
        "{probe}: payload size"
    );
    let word = |at: usize| u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes"));
    let dword = |at: usize| u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"));
    assert_eq!(word(4) as usize, cases, "{probe}: case count");
    match word(0) {
        DONE => {}
        PROGRESS => {
            return Capture::Ended {
                case: word(8) as usize,
            }
        }
        other => panic!("{probe}: frame status {other:#010x}"),
    }
    let runs = (0..cases)
        .map(|c| {
            [0, 1].map(|run| {
                let at = HEADER_BYTES + (2 * c + run) * RUN_BYTES;
                Run {
                    slots: [0, 1, 2, 3].map(|s| dword(at + 8 * s)),
                    xer: dword(at + 32),
                    cr: word(at + 40),
                    fpscr: dword(at + 48) as u32,
                }
            })
        })
        .collect();
    Capture::Done(runs)
}

/// CellGov's state after `case`'s words, or the outcome that stopped it.
pub fn cellgov_after(case: &Case, index: usize) -> Result<PpuStateSnapshot, Box<CaseOutcome>> {
    let mut s = case.initial(index);
    for &w in &case.words {
        s = execute_state(w, &s)?;
    }
    Ok(s)
}

/// Replay `case` against the console's `run`: the differential case
/// whose expected state is CellGov's own post-state with every
/// recorded component replaced by the console's.
pub fn replay(probe: &str, case: &Case, index: usize, run: &Run) -> CaseOutcome {
    let Some((&last, prefix)) = case.words.split_last() else {
        unreachable!("parse_table refuses a case with no words")
    };
    let mut before = case.initial(index);
    for &w in prefix {
        before = match execute_state(w, &before) {
            Ok(s) => s,
            Err(outcome) => return *outcome,
        };
    }
    let mut expected = match execute_state(last, &before) {
        Ok(s) => s,
        Err(outcome) => return *outcome,
    };
    for (slot, &r) in case.record.iter().enumerate() {
        r.write(&mut expected, run.slots[slot]);
    }
    expected.cr = run.cr;
    expected.xer = run.xer;
    expected.fpscr = run.fpscr;
    run_case(&InstructionCase {
        label: format!("{probe} case {index} {}", case.id),
        initial_state: before,
        initial_memory: MemorySnapshot::empty(),
        raw_instruction: last,
        expected_state: expected,
        expected_memory: MemorySnapshot::empty(),
        source: OracleSource::ConsoleCapture {
            capture_id: capture_id(probe),
        },
    })
}

/// A recorded component two console runs of one case disagree on.
pub fn varies(case: &Case, runs: &[Run; 2]) -> Vec<String> {
    let [a, b] = runs;
    let mut out = Vec::new();
    for (slot, &r) in case.record.iter().enumerate() {
        if a.slots[slot] != b.slots[slot] {
            out.push(r.name());
        }
    }
    for (name, differs) in [
        ("xer", a.xer != b.xer),
        ("cr", a.cr != b.cr),
        ("fpscr", a.fpscr != b.fpscr),
    ] {
        if differs {
            out.push(name.to_string());
        }
    }
    out
}

/// Mnemonics a table may name that CellGov does not decode yet. `addc`
/// is a fixed-point add carrying [PPC-Book1 p:53 s:3.3.8]; its console
/// answers wait here for the decoder.
const UNDECODED: [&str; 1] = ["addc"];

/// The probe's assembly text for each word, as CellGov renders it.
fn rendered(words: &[u32]) -> String {
    words
        .iter()
        .map(|&w| match decode(w) {
            Ok(insn) => AsmText {
                insn: &insn,
                addr: 0,
                symbols: None,
            }
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
            Err(e) => format!("<{e}>"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn each_probe_header_is_its_table_rendered() {
    let regenerate = std::env::var_os("CELLGOV_REGENERATE").is_some();
    for probe in PROBES {
        let want = render(probe, &table(probe));
        let path = probe_dir(probe).join("ppu/cases.h");
        if regenerate {
            std::fs::write(&path, &want).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        let have =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            have == want,
            "{}: stale; rerun with CELLGOV_REGENERATE=1",
            path.display()
        );
    }
}

#[test]
fn each_row_asm_text_is_its_words() {
    let mut rows = 0;
    let mut wrong = Vec::new();
    for probe in PROBES {
        for case in table(probe) {
            rows += 1;
            if case.may_end_process {
                continue;
            }
            let want = case
                .asm
                .split(';')
                .map(|w| w.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("; ");
            let have = rendered(&case.words);
            let mnemonic = want.split([' ', '.']).next().unwrap_or_default();
            if have.starts_with('<') && UNDECODED.contains(&mnemonic.trim_end_matches('o')) {
                continue;
            }
            if have != want {
                wrong.push(format!(
                    "{probe} {}: table {want:?}, words {have:?}",
                    case.id
                ));
            }
        }
    }
    assert!(rows > 0);
    assert!(
        wrong.is_empty(),
        "{} row(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_table_refuses_a_fatal_case_beside_another() {
    let text = "id\tasm\twords\tinputs\trecord\tmay_end_process\n\
                a\tnop\t0x60000000\t-\t-\t1\n\
                b\tnop\t0x60000000\t-\t-\t0\n";
    let err = parse_table(text).expect_err("two cases, one fatal");
    assert!(err.contains("only case"), "{err}");
}

#[test]
fn a_table_refuses_the_routine_registers() {
    for reg in ["r11", "r12", "r0", "r13", "f0", "f14"] {
        let text = format!(
            "id\tasm\twords\tinputs\trecord\tmay_end_process\na\tnop\t0x60000000\t-\t{reg}\t0\n"
        );
        let err = parse_table(&text).expect_err(reg);
        assert!(err.contains("not one a case may use"), "{reg}: {err}");
    }
}
