//! The console-captured reference vectors a trace record can be held
//! against, and which side of a disagreement each one supports.

use std::path::{Path, PathBuf};

use cellgov_ppu::differential::rpcs3_capture::CapturedRecord;
use cellgov_ppu::differential::{
    ComparedControlBits, ControlRegisters, OracleSource, PpuStateSnapshot,
};
use cellgov_ppu::instruction::fuzz::{generation_descriptor, PpuOperandClass};

use crate::ppu_reference::{
    parse_reference_file, PpuReferenceError, PpuReferenceFile, PpuReferenceInputState,
    PpuReferenceObservation, PpuReferenceProvenance, PpuReferenceState,
};
use crate::reference::ReferenceField;

/// A reference directory that [`console_vectors`] cannot load.
#[derive(Debug, thiserror::Error)]
pub enum PpuTraceReferenceError {
    /// A directory or file read failed.
    #[error("PPU trace reference read {}: {source}", path.display())]
    Read {
        /// The path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: std::io::Error,
    },
    /// A reference file did not parse or validate.
    #[error("PPU trace reference {file}: {source}")]
    Parse {
        /// The file name.
        file: String,
        /// The refusal.
        #[source]
        source: PpuReferenceError,
    },
}

/// One single-word vector a console captured.
#[derive(Debug, Clone)]
pub struct ConsoleVector {
    /// The reference file that holds it.
    pub file: String,
    /// The vector's name in that file.
    pub name: String,
    /// The instruction word.
    pub word: u32,
    /// The state the console started from.
    pub start: PpuStateSnapshot,
    /// The registers the vector loads, as `(gpr, fpr, vr)` indices.
    loaded: [Vec<u8>; 3],
    /// Base address of `memory`.
    pub memory_base: u64,
    /// The bytes the console started from.
    pub memory: Vec<u8>,
    /// What the console reported.
    pub expected: PpuReferenceObservation,
}

/// The side of a disagreement a console vector supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleSide {
    /// The console agrees with RPCS3 and not with CellGov.
    Rpcs3,
    /// The console agrees with CellGov and not with RPCS3.
    CellGov,
}

/// Every single-word, console-captured vector of the `.json` files in
/// `dir`, in file-name order.
///
/// # Errors
///
/// - [`PpuTraceReferenceError::Read`] when a directory or file read fails.
/// - [`PpuTraceReferenceError::Parse`] when a file does not parse.
pub fn console_vectors(dir: &Path) -> Result<Vec<ConsoleVector>, PpuTraceReferenceError> {
    let mut out = Vec::new();
    for name in json_names(dir)? {
        let path = dir.join(&name);
        let json = std::fs::read_to_string(&path).map_err(read_error(&path))?;
        let vectors =
            file_vectors(&name, &json).map_err(|source| PpuTraceReferenceError::Parse {
                file: name.clone(),
                source,
            })?;
        out.extend(vectors);
    }
    Ok(out)
}

/// The refusal for a failed read of `path`.
fn read_error(path: &Path) -> impl FnOnce(std::io::Error) -> PpuTraceReferenceError {
    let path = path.to_path_buf();
    move |source| PpuTraceReferenceError::Read { path, source }
}

/// The `.json` file names in `dir`, sorted.
fn json_names(dir: &Path) -> Result<Vec<String>, PpuTraceReferenceError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(read_error(dir))? {
        let name = entry.map_err(read_error(dir))?.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".json") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The single-word, console-captured vectors of the reference file
/// `name` whose text is `json`.
fn file_vectors(name: &str, json: &str) -> Result<Vec<ConsoleVector>, PpuReferenceError> {
    let mut out = Vec::new();
    match parse_reference_file(json)? {
        PpuReferenceFile::Single(artifact) => {
            let vector = ConsoleVector::new(
                name,
                &artifact.case_id,
                &artifact.provenance,
                &artifact.words,
                &artifact.initial_state,
                (artifact.memory_base, &artifact.initial_memory),
                &artifact.expected,
            )?;
            out.extend(vector);
        }
        PpuReferenceFile::Set(set) => {
            for v in &set.vectors {
                let vector = ConsoleVector::new(
                    name,
                    &v.name,
                    &v.provenance,
                    &v.words,
                    &v.initial_state,
                    (v.memory_base, &v.initial_memory),
                    &v.expected,
                )?;
                out.extend(vector);
            }
        }
    }
    Ok(out)
}

impl ConsoleVector {
    /// The vector, or `None` for one that runs more than one word or
    /// whose expected end no console captured.
    fn new(
        file: &str,
        name: &str,
        provenance: &PpuReferenceProvenance,
        words: &[u32],
        initial: &PpuReferenceInputState,
        (memory_base, memory): (u64, &[u8]),
        expected: &PpuReferenceObservation,
    ) -> Result<Option<Self>, PpuReferenceError> {
        let &[word] = words else {
            return Ok(None);
        };
        if !matches!(provenance, PpuReferenceProvenance::HardwareCapture { .. }) {
            return Ok(None);
        }
        let start = PpuStateSnapshot::capture(&initial.to_state()?);
        let loaded = [
            initial.gpr.keys().copied().collect(),
            initial.fpr.keys().copied().collect(),
            initial.vr_hex.keys().copied().collect(),
        ];
        Ok(Some(Self {
            file: file.to_owned(),
            name: name.to_owned(),
            word,
            start,
            loaded,
            memory_base,
            memory: memory.to_vec(),
            expected: expected.clone(),
        }))
    }

    /// Whether `record` ran this vector's word from this vector's inputs.
    ///
    /// The record must agree with the vector's start on:
    /// - every register the vector loads;
    /// - every register an operand field of the word names, in all three
    ///   banks, since the field does not say which bank it reads;
    /// - CR, LR, CTR, XER, the FPSCR, VRSAVE, the VSCR and the
    ///   reservation;
    /// - every byte of the record's storage window, which must lie in the
    ///   vector's memory.
    pub fn matches(&self, record: &CapturedRecord) -> bool {
        if record.raw_instruction != self.word {
            return false;
        }
        let pre = &record.pre_state;
        let start = &self.start;
        let named = operand_registers(self.word);
        let [gprs, fprs, vrs] = &self.loaded;
        let agree = |bank: &[u8], same: &dyn Fn(usize) -> bool| {
            bank.iter().chain(&named).all(|&r| same(usize::from(r)))
        };
        agree(gprs, &|r| pre.gpr[r] == start.gpr[r])
            && agree(fprs, &|r| pre.fpr[r] == start.fpr[r])
            && agree(vrs, &|r| pre.vr[r] == start.vr[r])
            && (pre.cr, pre.lr, pre.ctr, pre.xer) == (start.cr, start.lr, start.ctr, start.xer)
            && (pre.fpscr, pre.vrsave, pre.vscr) == (start.fpscr, start.vrsave, start.vscr)
            && pre.reservation == start.reservation
            && self.holds_window(record.mem_addr, &record.mem_pre)
    }

    /// Whether the vector's memory holds `bytes` at `addr`.
    fn holds_window(&self, addr: u64, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        let Some(offset) = addr.checked_sub(self.memory_base) else {
            return false;
        };
        let Ok(offset) = usize::try_from(offset) else {
            return false;
        };
        offset
            .checked_add(bytes.len())
            .and_then(|end| self.memory.get(offset..end))
            == Some(bytes)
    }

    /// The side the console supports when RPCS3 ended in `rpcs3` and
    /// CellGov in `cellgov`, or `None` when the console decides nothing.
    ///
    /// The console decides nothing when it:
    /// - agrees with both sides or with neither;
    /// - does not judge every field the two sides differ on.
    ///
    /// The console judges a register only when the vector holds it: the
    /// vector loads it, or an operand field of the word names it.
    /// [`Self::matches`] leaves every other register free, so the
    /// console's end there says nothing about the record.
    ///
    /// The console judges RPCS3's end only on the control-register bits
    /// the trace holds from RPCS3's own model, per `control`: the other
    /// bits keep their values from before the word, so a console end
    /// that sets them says nothing about RPCS3.
    ///
    /// The storage arguments are each side's bytes of the record's window
    /// at `addr`.
    pub fn decide(
        &self,
        rpcs3: (&PpuStateSnapshot, &[u8]),
        cellgov: (&PpuStateSnapshot, &[u8]),
        addr: u64,
        control: ControlRegisters,
    ) -> Option<ConsoleSide> {
        let held = self.held_registers();
        let agrees = |(state, bytes): (&PpuStateSnapshot, &[u8]),
                      bits: Option<ComparedControlBits>| {
            state_agrees(&self.expected.state, state, &held, bits)
                && self.memory_agrees(addr, bytes)
        };
        let every_bit = ComparedControlBits {
            fpscr: u32::MAX,
            vscr: u32::MAX,
        };
        match (
            agrees(rpcs3, rpcs3_control_bits(control)),
            agrees(cellgov, Some(every_bit)),
        ) {
            (true, false) => Some(ConsoleSide::Rpcs3),
            (false, true) if self.unjudged_equal(rpcs3, cellgov, addr, &held) => {
                Some(ConsoleSide::CellGov)
            }
            _ => None,
        }
    }

    /// The registers the console's end is held to, per bank, as
    /// `(gpr, fpr, vr)` masks.
    fn held_registers(&self) -> [[bool; 32]; 3] {
        let named = operand_registers(self.word);
        let mut held = [[false; 32]; 3];
        for (mask, loaded) in held.iter_mut().zip(&self.loaded) {
            for &r in loaded.iter().chain(&named) {
                if let Some(slot) = mask.get_mut(usize::from(r)) {
                    *slot = true;
                }
            }
        }
        held
    }

    /// Whether the two sides agree on every field the console does not
    /// judge.
    fn unjudged_equal(
        &self,
        (a, a_bytes): (&PpuStateSnapshot, &[u8]),
        (b, b_bytes): (&PpuStateSnapshot, &[u8]),
        addr: u64,
        [gprs, fprs, vrs]: &[[bool; 32]; 3],
    ) -> bool {
        let e = &self.expected.state;
        let bytes = a_bytes.len() == b_bytes.len()
            && a_bytes
                .iter()
                .zip(b_bytes)
                .enumerate()
                .all(|(i, (x, y))| x == y || self.console_byte(addr, i).is_some());
        bank_unjudged_equal(&e.gpr, gprs, &a.gpr, &b.gpr)
            && bank_unjudged_equal(&e.fpr, fprs, &a.fpr, &b.fpr)
            && bank_unjudged_equal(&e.vr_hex, vrs, &a.vr, &b.vr)
            && (judged(&e.cr) || a.cr == b.cr)
            && (judged(&e.lr) || a.lr == b.lr)
            && (judged(&e.ctr) || a.ctr == b.ctr)
            && (judged(&e.xer) || a.xer == b.xer)
            && (judged(&e.fpscr) || a.fpscr == b.fpscr)
            && (judged(&e.vrsave) || a.vrsave == b.vrsave)
            && (judged(&e.vscr) || a.vscr == b.vscr)
            && (judged(&e.reservation) || a.reservation == b.reservation)
            && bytes
    }

    /// The console's end byte at offset `i` of the window at `addr`, or
    /// `None` when the console reports no byte there.
    fn console_byte(&self, addr: u64, i: usize) -> Option<u8> {
        let ReferenceField::Value { value } = &self.expected.memory else {
            return None;
        };
        let at = addr.wrapping_add(i as u64).wrapping_sub(self.memory_base);
        usize::try_from(at)
            .ok()
            .and_then(|at| value.get(at))
            .copied()
    }

    /// Whether every byte of `bytes` at `addr` that the console's memory
    /// reports matches it.
    fn memory_agrees(&self, addr: u64, bytes: &[u8]) -> bool {
        bytes.iter().enumerate().all(|(i, &byte)| {
            self.console_byte(addr, i)
                .is_none_or(|console| console == byte)
        })
    }
}

/// Whether the console reports `field`.
fn judged<T>(field: &ReferenceField<T>) -> bool {
    matches!(field, ReferenceField::Value { .. })
}

/// Whether `a` and `b` agree at every bank index the console does not
/// judge.
///
/// The console judges no index when it reports no bank value, and no
/// index that `held` leaves out.
fn bank_unjudged_equal<T, U: PartialEq>(
    field: &ReferenceField<Vec<T>>,
    held: &[bool; 32],
    a: &[U],
    b: &[U],
) -> bool {
    let reported = judged(field);
    held.iter()
        .zip(a.iter().zip(b))
        .all(|(&h, (x, y))| (reported && h) || x == y)
}

/// The register numbers the operand fields of `word` hold.
fn operand_registers(word: u32) -> Vec<u8> {
    let Some(descriptor) = generation_descriptor(word) else {
        return Vec::new();
    };
    descriptor
        .operands
        .iter()
        .filter(|field| field.class == PpuOperandClass::Register)
        .map(|field| ((word & field.mask) >> field.mask.trailing_zeros()) as u8)
        .collect()
}

/// The FPSCR and VSCR bits an RPCS3 end is judged on, or `None` when the
/// trace recorded neither them nor VRSAVE.
fn rpcs3_control_bits(control: ControlRegisters) -> Option<ComparedControlBits> {
    // The runner's own rule for an RPCS3 capture, so a bit it does not
    // compare is a bit this check does not judge either.
    let source = OracleSource::Rpcs3Capture {
        capture_id: "rpcs3-trace",
        control_registers: control,
    };
    (source.control_registers() == ControlRegisters::Recorded)
        .then(|| source.compared_control_bits())
}

/// Whether `state` matches every state field the console reports.
///
/// A register bank compares only at the registers `held` names. The
/// FPSCR and the VSCR compare only at the bits `control` names. A
/// `control` of `None` judges no FPSCR, VSCR or VRSAVE.
fn state_agrees(
    expected: &PpuReferenceState,
    state: &PpuStateSnapshot,
    [gprs, fprs, vrs]: &[[bool; 32]; 3],
    control: Option<ComparedControlBits>,
) -> bool {
    let vr_hex: Vec<String> = state.vr.iter().map(|v| format!("{v:032x}")).collect();
    same_held(&expected.gpr, gprs, &state.gpr)
        && same_held(&expected.fpr, fprs, &state.fpr)
        && same_held(&expected.vr_hex, vrs, &vr_hex)
        && same_one(&expected.cr, &state.cr)
        && same_one(&expected.lr, &state.lr)
        && same_one(&expected.ctr, &state.ctr)
        && same_one(&expected.xer, &state.xer)
        && control.is_none_or(|bits| {
            same_bits(&expected.fpscr, state.fpscr, bits.fpscr)
                && same_one(&expected.vrsave, &state.vrsave)
                && same_bits(&expected.vscr, state.vscr, bits.vscr)
        })
        && same_one(
            &expected.reservation,
            &state.reservation.map(|line| line.addr()),
        )
}

/// Whether `observed` matches the bank `field` at every held index, or
/// `field` reports no value.
fn same_held<T: PartialEq>(
    field: &ReferenceField<Vec<T>>,
    held: &[bool; 32],
    observed: &[T],
) -> bool {
    let ReferenceField::Value { value } = field else {
        return true;
    };
    held.iter()
        .zip(observed)
        .enumerate()
        .filter(|&(_, (&h, _))| h)
        .all(|(i, (_, o))| value.get(i) == Some(o))
}

/// Whether `observed` matches `field` at the bits of `mask`, or `field`
/// reports no value.
fn same_bits(field: &ReferenceField<u32>, observed: u32, mask: u32) -> bool {
    match field {
        ReferenceField::Value { value } => (value ^ observed) & mask == 0,
        _ => true,
    }
}

/// Whether `observed` matches `field`, or `field` reports no value.
fn same_one<T: PartialEq>(field: &ReferenceField<T>, observed: &T) -> bool {
    match field {
        ReferenceField::Value { value } => value == observed,
        _ => true,
    }
}
