//! A documented PPU vector: the instructions, the start state, and what
//! the documents say each recorded component of the end state is.

use serde::Serialize;

use super::set::Set;

/// Where a component lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    /// A general-purpose register.
    Gpr(u8),
    /// A floating-point register.
    Fpr(u8),
    /// A vector register.
    Vr(u8),
    /// The big-endian doubleword at this byte offset of the memory
    /// buffer.
    Mem(u16),
    /// The condition register.
    Cr,
    /// The fixed-point exception register.
    Xer,
    /// The floating-point status and control register.
    Fpscr,
}

impl Location {
    /// The name the probe's case table uses: `r3`, `f1`, `v2`, `m8`,
    /// `cr`, `xer`, `fpscr`.
    pub fn name(self) -> String {
        match self {
            Self::Gpr(n) => format!("r{n}"),
            Self::Fpr(n) => format!("f{n}"),
            Self::Vr(n) => format!("v{n}"),
            Self::Mem(off) => format!("m{off}"),
            Self::Cr => "cr".to_owned(),
            Self::Xer => "xer".to_owned(),
            Self::Fpscr => "fpscr".to_owned(),
        }
    }

    /// Whether the probe records the location in a slot of its own; the
    /// probe records CR, XER and the FPSCR for every vector.
    pub fn takes_slot(self) -> bool {
        matches!(
            self,
            Self::Gpr(_) | Self::Fpr(_) | Self::Vr(_) | Self::Mem(_)
        )
    }
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
        /// The page that leaves them open.
        citation: &'static str,
    },
}

/// One masked piece of the end state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// A stable name, as `r3.hi` or `cr0.so`.
    pub name: String,
    /// The register or memory doubleword.
    pub location: Location,
    /// The bits of the location the verdict covers.
    pub mask: u128,
    /// What the documents say the bits are.
    pub verdict: Verdict,
}

/// One vector of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentedVector {
    /// A stable id, as `muldiv-undef/divw/zero/oe1-rc1-so0`.
    pub id: String,
    /// The set the vector belongs to.
    pub set: Set,
    /// The instructions as assembly text, `; ` between them.
    pub asm: String,
    /// The instruction words, run in order.
    pub words: Vec<u32>,
    /// The registers loaded before the words, in order; every recorded
    /// register is among them.
    pub inputs: Vec<(Location, u128)>,
    /// CR before the words.
    pub cr: u32,
    /// XER before the words.
    pub xer: u64,
    /// The FPSCR before the words.
    pub fpscr: u32,
    /// The memory buffer before the words, or `None`.
    /// [`MEM_BASE_GPR`](super::MEM_BASE_GPR) holds its address.
    pub mem: Option<Vec<u8>>,
    /// Every component of the end state the vector checks.
    pub expected: Vec<Component>,
}

impl DocumentedVector {
    /// The locations the probe records in slots, in first-named order.
    pub fn record(&self) -> Vec<Location> {
        let mut out: Vec<Location> = Vec::new();
        for c in &self.expected {
            if c.location.takes_slot() && !out.contains(&c.location) {
                out.push(c.location);
            }
        }
        out
    }

    /// The vector's row of the probe's case table:
    /// `id asm words inputs record may_end_process`.
    pub fn case_row(&self) -> String {
        let words: Vec<String> = self.words.iter().map(|w| format!("0x{w:08x}")).collect();
        let mut inputs = vec![
            format!("cr=0x{:x}", self.cr),
            format!("xer=0x{:x}", self.xer),
            format!("fpscr=0x{:x}", self.fpscr),
        ];
        for &(location, value) in &self.inputs {
            inputs.push(format!("{}=0x{value:x}", location.name()));
        }
        if let Some(mem) = &self.mem {
            let hex: String = mem.iter().map(|b| format!("{b:02x}")).collect();
            inputs.push(format!("mem=0x{hex}"));
        }
        let record: Vec<String> = self.record().iter().map(|l| l.name()).collect();
        let record = if record.is_empty() {
            "-".to_owned()
        } else {
            record.join(",")
        };
        format!(
            "{}\t{}\t{}\t{}\t{}\t0",
            self.id,
            self.asm,
            words.join(","),
            inputs.join(","),
            record
        )
    }
}

/// The probe's case table for the whole list.
pub fn case_table(vectors: &[DocumentedVector]) -> String {
    let mut text = String::from("id\tasm\twords\tinputs\trecord\tmay_end_process\n");
    for vector in vectors {
        text.push_str(&vector.case_row());
        text.push('\n');
    }
    text
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
        citation: &'static str,
        reason: &'static str,
    },
}

#[derive(Serialize)]
struct WireVector<'a> {
    id: &'a str,
    set: Set,
    provenance: &'static str,
    asm: &'a str,
    words: Vec<String>,
    inputs: Vec<(String, String)>,
    cr: String,
    xer: String,
    fpscr: String,
    mem: Option<String>,
    expected: Vec<WireComponent<'a>>,
}

fn hex128(value: u128) -> String {
    format!("{value:032x}")
}

/// The vector as one line of JSON.
///
/// Words and CR are eight hex digits, XER sixteen; registers, masks and
/// values are 32 hex digits, most significant first.
///
/// # Errors
///
/// A `serde_json::Error` when serialization fails.
pub fn to_json_line(vector: &DocumentedVector) -> Result<String, serde_json::Error> {
    let wire = WireVector {
        id: &vector.id,
        set: vector.set,
        provenance: "documented_vector",
        asm: &vector.asm,
        words: vector.words.iter().map(|w| format!("{w:08x}")).collect(),
        inputs: vector
            .inputs
            .iter()
            .map(|&(l, v)| (l.name(), hex128(v)))
            .collect(),
        cr: format!("{:08x}", vector.cr),
        xer: format!("{:016x}", vector.xer),
        fpscr: format!("{:08x}", vector.fpscr),
        mem: vector
            .mem
            .as_ref()
            .map(|m| m.iter().map(|b| format!("{b:02x}")).collect()),
        expected: vector
            .expected
            .iter()
            .map(|c| WireComponent {
                name: &c.name,
                at: c.location.name(),
                mask: hex128(c.mask),
                verdict: match c.verdict {
                    Verdict::Value { value, citation } => WireVerdict::Value {
                        value: hex128(value & c.mask),
                        citation,
                    },
                    Verdict::Undefined { citation } => WireVerdict::Undefined {
                        citation,
                        reason: vector.set.reason(),
                    },
                },
            })
            .collect(),
    };
    serde_json::to_string(&wire)
}

/// The whole list as JSON lines, one vector per line.
///
/// # Errors
///
/// The first error [`to_json_line`] returns.
pub fn render(vectors: &[DocumentedVector]) -> Result<String, serde_json::Error> {
    let mut text = String::new();
    for vector in vectors {
        text.push_str(&to_json_line(vector)?);
        text.push('\n');
    }
    Ok(text)
}
