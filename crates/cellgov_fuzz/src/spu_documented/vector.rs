//! A documented vector: the instructions, the start state, and what the
//! documents say each component of the end state is.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;
use serde::Serialize;

use super::model::{effect, fpscr_bit, Insn, Known};
use super::question::{Question, VectorClass};

/// Where a component lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    /// A general-purpose register.
    Register(u8),
    /// The floating-point status and control register.
    Fpscr,
}

/// What the documents say a component is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The documents fix the bits under the mask.
    Value {
        /// The bits, read under the component's mask.
        value: u128,
        /// The page that fixes them.
        citation: &'static str,
    },
    /// The documents leave the bits open.
    Undefined {
        /// The open question.
        question: Question,
    },
}

/// One masked piece of the end state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// A stable name, as `r3.w1` or `fpscr.s0.ovf`.
    pub name: String,
    /// The register or the FPSCR.
    pub location: Location,
    /// The bits of the 128-bit location the verdict covers.
    pub mask: u128,
    /// What the documents say the bits are.
    pub verdict: Verdict,
}

/// One vector of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentedVector {
    /// A stable id, as `fma/cell/0042`.
    pub id: String,
    /// The row of the list the vector belongs to.
    pub class: VectorClass,
    /// The coverage-model cells the vector's slots reach.
    pub cells: Vec<String>,
    /// The instructions, loaded from local-store address 0.
    pub words: Vec<u32>,
    /// The registers the instructions read; every other register is 0.
    pub registers: BTreeMap<u8, u128>,
    /// The FPSCR before the first instruction.
    pub fpscr: u128,
    /// Every component of the end state the vector checks.
    pub expected: Vec<Component>,
}

impl DocumentedVector {
    /// The questions the vector's undefined components reach.
    pub fn questions(&self) -> impl Iterator<Item = Question> + '_ {
        self.expected.iter().filter_map(|c| match c.verdict {
            Verdict::Undefined { question } => Some(question),
            Verdict::Value { .. } => None,
        })
    }
}

/// [SPU-ISA p:200 s:9.3] the status bits are sticky: once set they stay set until an fscrwr clears them, and no other instruction clears them.
const STICKY: &str = "SPU-ISA p:200 s:9.3";

/// The vector `insns` make from `registers` and `fpscr`.
///
/// `None` when an instruction before the last leaves a piece open,
/// because the next instruction would read an unknown value.
pub(crate) fn build(
    id: String,
    class: VectorClass,
    cells: Vec<String>,
    insns: &[Insn],
    registers: BTreeMap<u8, u128>,
    fpscr: u128,
) -> Option<DocumentedVector> {
    let (last, earlier) = insns.split_last()?;
    let mut regs = registers.clone();
    let mut raised = 0u128;
    let mut named = 0u128;
    let mut written: BTreeMap<u8, &'static str> = BTreeMap::new();
    for &insn in earlier {
        let step = effect(insn, &regs, fpscr | raised);
        for write in &step.writes {
            let Known::Value(value) = write.value else {
                return None;
            };
            let old = regs.get(&write.register).copied().unwrap_or(0);
            regs.insert(write.register, old & !write.mask | value & write.mask);
            written.insert(write.register, write.citation);
        }
        for flag in &step.flags {
            let Known::Value(set) = flag.raised else {
                return None;
            };
            named |= fpscr_bit(flag.bit);
            if set {
                raised |= fpscr_bit(flag.bit);
            }
        }
    }
    let step = effect(*last, &regs, fpscr | raised);
    let mut expected = Vec::new();
    for (&register, &citation) in &written {
        if step.writes.iter().any(|w| w.register == register) {
            continue;
        }
        expected.push(Component {
            name: format!("r{register}"),
            location: Location::Register(register),
            mask: u128::MAX,
            verdict: Verdict::Value {
                value: regs[&register],
                citation,
            },
        });
    }
    for write in &step.writes {
        expected.push(Component {
            name: format!("r{}.{}", write.register, write.piece),
            location: Location::Register(write.register),
            mask: write.mask,
            verdict: match write.value {
                Known::Value(value) => Verdict::Value {
                    value,
                    citation: write.citation,
                },
                Known::Open(question) => Verdict::Undefined { question },
            },
        });
    }
    for flag in &step.flags {
        let bit = fpscr_bit(flag.bit);
        named |= bit;
        let before = (fpscr | raised) & bit != 0;
        let verdict = match flag.raised {
            _ if before => Verdict::Value {
                value: bit,
                citation: STICKY,
            },
            Known::Value(set) => Verdict::Value {
                value: if set { bit } else { 0 },
                citation: flag.citation,
            },
            Known::Open(question) => Verdict::Undefined { question },
        };
        expected.push(Component {
            name: format!("fpscr.{}", flag.name),
            location: Location::Fpscr,
            mask: bit,
            verdict,
        });
    }
    // Bits an earlier instruction raised and the last one does not name.
    let earlier_only = named & !step.flags.iter().fold(0, |m, f| m | fpscr_bit(f.bit));
    let rest = FPSCR_DEFINED & !named | earlier_only;
    expected.push(Component {
        name: "fpscr.rest".to_owned(),
        location: Location::Fpscr,
        mask: rest,
        verdict: Verdict::Value {
            value: (fpscr | raised) & rest,
            citation: STICKY,
        },
    });
    Some(DocumentedVector {
        id,
        class,
        cells,
        words: insns.iter().map(|i| i.encode()).collect(),
        registers,
        fpscr,
        expected,
    })
}

#[derive(Serialize)]
struct WireComponent<'a> {
    name: &'a str,
    at: String,
    mask: String,
    #[serde(flatten)]
    verdict: WireVerdict,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WireVerdict {
    Value {
        value: String,
        citation: &'static str,
    },
    Undefined {
        question: Question,
        citation: &'static str,
        reason: &'static str,
    },
}

#[derive(Serialize)]
struct WireVector<'a> {
    id: &'a str,
    class: VectorClass,
    cells: &'a [String],
    words: Vec<String>,
    registers: BTreeMap<String, String>,
    fpscr: String,
    expected: Vec<WireComponent<'a>>,
}

fn hex128(value: u128) -> String {
    format!("{value:032x}")
}

/// The vector as one line of JSON.
///
/// Words are eight hex digits. Registers, masks, values and the FPSCR
/// are 32 hex digits, most significant first.
pub fn to_json_line(vector: &DocumentedVector) -> Result<String, serde_json::Error> {
    let wire = WireVector {
        id: &vector.id,
        class: vector.class,
        cells: &vector.cells,
        words: vector.words.iter().map(|w| format!("{w:08x}")).collect(),
        registers: vector
            .registers
            .iter()
            .map(|(r, v)| (r.to_string(), hex128(*v)))
            .collect(),
        fpscr: hex128(vector.fpscr),
        expected: vector
            .expected
            .iter()
            .map(|c| WireComponent {
                name: &c.name,
                at: match c.location {
                    Location::Register(r) => format!("r{r}"),
                    Location::Fpscr => "fpscr".to_owned(),
                },
                mask: hex128(c.mask),
                verdict: match c.verdict {
                    Verdict::Value { value, citation } => WireVerdict::Value {
                        value: hex128(value & c.mask),
                        citation,
                    },
                    Verdict::Undefined { question } => WireVerdict::Undefined {
                        question,
                        citation: question.citation(),
                        reason: question.reason(),
                    },
                },
            })
            .collect(),
    };
    serde_json::to_string(&wire)
}

/// The whole list as JSON lines, one vector per line.
pub fn render(vectors: &[DocumentedVector]) -> Result<String, serde_json::Error> {
    let mut text = String::new();
    for vector in vectors {
        text.push_str(&to_json_line(vector)?);
        text.push('\n');
    }
    Ok(text)
}

/// FNV-1a over `text`: the pin that every host renders the same list.
pub fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
