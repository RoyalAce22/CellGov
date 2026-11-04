//! The case table: one row per case. The parser checks each row against
//! the registers and memory the probe routine leaves free for a case.

use std::path::{Path, PathBuf};

use cellgov_ppu::differential::PpuStateSnapshot;

/// The reference console profile every capture here is read from.
pub const PROFILE: &str = "cech20-cex-493";

/// The GPRs a case may load or record; r0, r11 and r12 are the
/// routine's own, and r10 holds the memory buffer's address in a case
/// that has one.
const GPRS: std::ops::RangeInclusive<u8> = 3..=10;

/// The FPRs a case may load or record; f0 is the routine's own.
const FPRS: std::ops::RangeInclusive<u8> = 1..=13;

/// The VRs a case may load or record.
const VRS: std::ops::RangeInclusive<u8> = 1..=13;

/// The 8-byte record slots of one run.
pub(super) const SLOTS: usize = 4;

/// Bytes of the memory buffer a case may name: two reservation granules.
pub(super) const MEM_BYTES: usize = 256;

/// The GPR that holds the memory buffer's address.
pub(super) const MEM_BASE_GPR: u8 = 10;

/// A register or memory doubleword a case loads or records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    /// GPR `n`.
    Gpr(u8),
    /// FPR `n`.
    Fpr(u8),
    /// VR `n`; it takes two record slots.
    Vr(u8),
    /// The memory doubleword at this byte offset of the buffer.
    Mem(u16),
}

impl Reg {
    fn parse(text: &str) -> Result<Self, String> {
        let refuse = || format!("register {text:?} is not one a case may use");
        let (kind, n) = text.split_at_checked(1).ok_or_else(refuse)?;
        let n: u16 = n.parse().map_err(|_| refuse())?;
        let small = u8::try_from(n).ok();
        match (kind, small) {
            ("r", Some(n)) if GPRS.contains(&n) => Ok(Reg::Gpr(n)),
            ("f", Some(n)) if FPRS.contains(&n) => Ok(Reg::Fpr(n)),
            ("v", Some(n)) if VRS.contains(&n) => Ok(Reg::Vr(n)),
            ("m", _) if n.is_multiple_of(8) && usize::from(n) < MEM_BYTES => Ok(Reg::Mem(n)),
            _ => Err(refuse()),
        }
    }

    /// The record slots the register takes.
    pub(super) fn width(self) -> usize {
        match self {
            Reg::Vr(_) => 2,
            _ => 1,
        }
    }

    /// Writes `v` into `s`; does nothing for a memory doubleword, which
    /// `s` does not hold.
    pub(super) fn write(self, s: &mut PpuStateSnapshot, v: u128) {
        match self {
            Reg::Gpr(n) => s.gpr[usize::from(n)] = v as u64,
            Reg::Fpr(n) => s.fpr[usize::from(n)] = v as u64,
            Reg::Vr(n) => s.vr[usize::from(n)] = v,
            Reg::Mem(_) => {}
        }
    }

    /// The table's name for the register.
    pub fn name(self) -> String {
        match self {
            Reg::Gpr(n) => format!("r{n}"),
            Reg::Fpr(n) => format!("f{n}"),
            Reg::Vr(n) => format!("v{n}"),
            Reg::Mem(off) => format!("m{off}"),
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
    pub inputs: Vec<(Reg, u128)>,
    /// CR before the words.
    pub cr: u32,
    /// XER before the words.
    pub xer: u64,
    /// FPSCR before the words.
    pub fpscr: u32,
    /// The memory buffer before the words, zero-padded to
    /// [`MEM_BYTES`]; `None` when the case names no memory.
    pub mem: Option<Vec<u8>>,
    /// The registers recorded after the words, in table order.
    pub record: Vec<Reg>,
    /// Whether the case may end the process.
    pub may_end_process: bool,
}

impl Case {
    /// The value a recorded register holds before the words when no
    /// input names it: distinct per case and register, so an unchanged
    /// register shows.
    fn preset(&self, index: usize, reg: Reg) -> u128 {
        let tag = match reg {
            Reg::Gpr(n) => u64::from(n),
            Reg::Fpr(n) => 0x80 | u64::from(n),
            Reg::Vr(n) => 0x40 | u64::from(n),
            Reg::Mem(_) => unreachable!("memory takes its value from the case's buffer"),
        };
        let word = 0x5a5a_0000_0000_0000 | ((index as u64) << 8) | tag;
        match reg {
            Reg::Vr(_) => u128::from(word) << 64 | u128::from(!word),
            _ => u128::from(word),
        }
    }

    /// Every register loaded before the words: the inputs, then a preset
    /// for each recorded GPR, FPR or VR that no input names.
    pub(super) fn loads(&self, index: usize) -> Vec<(Reg, u128)> {
        let mut out = self.inputs.clone();
        for &r in &self.record {
            if !matches!(r, Reg::Mem(_)) && !out.iter().any(|&(i, _)| i == r) {
                out.push((r, self.preset(index, r)));
            }
        }
        out
    }

    /// Each recorded register and its first record slot.
    ///
    /// A VR starts on an even slot, so `stvx` stores it 16-byte aligned.
    pub(super) fn slots(&self) -> Vec<(Reg, usize)> {
        let mut next = 0;
        self.record
            .iter()
            .map(|&r| {
                if r.width() == 2 && next % 2 == 1 {
                    next += 1;
                }
                let at = next;
                next += r.width();
                (r, at)
            })
            .collect()
    }

    /// The state the words start from, with the memory buffer at
    /// `mem_base`.
    pub fn initial(&self, index: usize, mem_base: u64) -> PpuStateSnapshot {
        let mut s = PpuStateSnapshot::zero();
        for (r, v) in self.loads(index) {
            r.write(&mut s, v);
        }
        if self.mem.is_some() {
            s.gpr[usize::from(MEM_BASE_GPR)] = mem_base;
        }
        s.cr = self.cr;
        s.xer = self.xer;
        s.fpscr = self.fpscr;
        s
    }
}

fn hex(text: &str) -> Result<u128, String> {
    let digits = text
        .strip_prefix("0x")
        .ok_or_else(|| format!("{text:?} is not 0x-prefixed hex"))?;
    u128::from_str_radix(&digits.replace('_', ""), 16).map_err(|e| format!("{text:?}: {e}"))
}

fn hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    let digits = text
        .strip_prefix("0x")
        .ok_or_else(|| format!("{text:?} is not 0x-prefixed hex"))?;
    if !digits.is_ascii() {
        return Err(format!("mem {text:?} is not hex"));
    }
    if digits.len() % 2 != 0 || digits.len() / 2 > MEM_BYTES {
        return Err(format!("mem holds {} hex digits", digits.len()));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|e| format!("mem: {e}")))
        .collect()
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
            .map(|w| hex(w).and_then(|v| u32::try_from(v).map_err(|e| e.to_string())))
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
            mem: None,
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
            if key == "mem" {
                if case.mem.is_some() {
                    return Err(at("mem loaded twice".into()));
                }
                let mut bytes = hex_bytes(value).map_err(at)?;
                bytes.resize(MEM_BYTES, 0);
                case.mem = Some(bytes);
                continue;
            }
            let value = hex(value).map_err(at)?;
            let narrow32 = |v: u128| u32::try_from(v).map_err(|_| at(format!("{key} {v:#x}")));
            let narrow64 = |v: u128| u64::try_from(v).map_err(|_| at(format!("{key} {v:#x}")));
            match key {
                "cr" => case.cr = narrow32(value)?,
                "xer" => case.xer = narrow64(value)?,
                "fpscr" => case.fpscr = narrow32(value)?,
                reg => {
                    let reg = Reg::parse(reg).map_err(at)?;
                    if matches!(reg, Reg::Mem(_)) {
                        return Err(at(format!("{key}: memory is loaded with mem=")));
                    }
                    if !matches!(reg, Reg::Vr(_)) {
                        narrow64(value)?;
                    }
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
        if case.mem.is_none() && case.record.iter().any(|r| matches!(r, Reg::Mem(_))) {
            return Err(at("a memory record needs mem=".into()));
        }
        let base = Reg::Gpr(MEM_BASE_GPR);
        if case.mem.is_some()
            && (case.record.contains(&base) || case.inputs.iter().any(|&(r, _)| r == base))
        {
            return Err(at(format!(
                "r{MEM_BASE_GPR} holds the memory buffer's address"
            )));
        }
        let used = case.slots().last().map_or(0, |&(r, at)| at + r.width());
        if used > SLOTS {
            return Err(at(format!("the records take {used} slots")));
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
