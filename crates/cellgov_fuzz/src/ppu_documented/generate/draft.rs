//! A vector under construction, and the values the sets share.

use crate::ppu_documented::encode::{join, Insn};
use crate::ppu_documented::set::Set;
use crate::ppu_documented::vector::{Component, DocumentedVector, Location, Verdict};

/// RT before the words when the vector writes RT: a pattern no result
/// of the list takes, so a target left unchanged shows.
pub(crate) const RT_PRESET: u64 = 0x5a5a_a5a5_5a5a_a5a5;

/// A VR target before the words.
pub(crate) const VR_PRESET: u128 = 0x5a5a_a5a5_5a5a_a5a5_a5a5_5a5a_a5a5_5a5a;

/// CR before the words: every field nonzero and distinct.
pub(crate) const CR_PRESET: u32 = 0x9abc_def0;

/// CR fields 1 to 7.
pub(crate) const CR_REST: u128 = 0x0fff_ffff;

/// The low word of a 64-bit register.
pub(crate) const LO: u128 = 0xffff_ffff;

/// The high word of a 64-bit register.
pub(crate) const HI: u128 = 0xffff_ffff_0000_0000;

/// A whole 64-bit register.
pub(crate) const DWORD: u128 = u64::MAX as u128;

/// The memory buffer before the words: byte `i` is `i ^ 0xa5`, so each
/// byte names its offset.
pub(crate) fn buffer(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8) ^ 0xa5).collect()
}

/// The big-endian doubleword of `bytes` at `off`.
pub(crate) fn dword_at(bytes: &[u8], off: usize) -> u128 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[off..off + 8]);
    u128::from(u64::from_be_bytes(word))
}

/// A vector under construction.
pub(crate) struct Draft {
    vector: DocumentedVector,
}

impl Draft {
    /// A vector of `set` named `<set>/<id>` that runs `insns`, from CR
    /// [`CR_PRESET`] and XER and FPSCR 0.
    pub(crate) fn new(set: Set, id: &str, insns: &[Insn]) -> Self {
        let (asm, words) = join(insns);
        Self {
            vector: DocumentedVector {
                id: format!("{}/{id}", set.id()),
                set,
                asm,
                words,
                inputs: Vec::new(),
                cr: CR_PRESET,
                xer: 0,
                fpscr: 0,
                mem: None,
                expected: Vec::new(),
            },
        }
    }

    /// Load `value` into the register `at` before the words.
    pub(crate) fn input(mut self, at: Location, value: u128) -> Self {
        self.vector.inputs.push((at, value));
        self
    }

    /// XER before the words.
    pub(crate) fn xer(mut self, xer: u64) -> Self {
        self.vector.xer = xer;
        self
    }

    /// The FPSCR before the words.
    pub(crate) fn fpscr(mut self, fpscr: u32) -> Self {
        self.vector.fpscr = fpscr;
        self
    }

    /// The memory buffer before the words.
    pub(crate) fn mem(mut self, bytes: Vec<u8>) -> Self {
        self.vector.mem = Some(bytes);
        self
    }

    /// A component the documents fix.
    pub(crate) fn value(
        mut self,
        name: &str,
        at: Location,
        mask: u128,
        value: u128,
        citation: &'static str,
    ) -> Self {
        self.vector.expected.push(Component {
            name: format!("{}.{name}", at.name()),
            location: at,
            mask,
            verdict: Verdict::Value {
                value: value & mask,
                citation,
            },
        });
        self
    }

    /// A component the documents leave open.
    pub(crate) fn undefined(
        mut self,
        name: &str,
        at: Location,
        mask: u128,
        citation: &'static str,
    ) -> Self {
        self.vector.expected.push(Component {
            name: format!("{}.{name}", at.name()),
            location: at,
            mask,
            verdict: Verdict::Undefined { citation },
        });
        self
    }

    /// The finished vector.
    pub(crate) fn finish(self) -> DocumentedVector {
        self.vector
    }
}
