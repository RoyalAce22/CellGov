//! The sequence-relation catalog as a generated reference: one entry per
//! row, written as Markdown and as JSON, with the stored counterexamples as
//! start states.
//!
//! [Necula2000 p:1 s:1] Translation validation checks each translation
//! against its source program. [Necula2000 p:2 s:1] A test runs a program
//! on a few inputs whose output is known. A row is a test: it holds on the
//! start states the campaigns ran.
//! [Lopes2021 p:65 s:1] A target refines a source when, for every input
//! state, it shows a subset of the source's behaviors. A row with a dead set
//! claims refinement, since its partner may leave any value there.

use std::fmt::Write as _;

use cellgov_spu::fuzz::{
    isa_citation, sequence_relations, SpuFloatClass, SpuSequencePartner, SpuSequencePin,
    SpuSequenceRelation, SpuSymbolicWord, SEQUENCE_PROGRAM_BASE, SEQUENCE_TAKEN_LANDING,
};

use super::counterexample::{
    component_name, stored_counterexamples, CounterexampleJson, RelationCounterexample, COMPONENTS,
};
use super::sequence_relations::{TAKEN_TERMINATOR, TERMINATOR};
use crate::error::{FuzzError, InvariantError};

/// Schema version of the catalog's JSON file.
pub(crate) const CATALOG_SCHEMA_VERSION: u32 = 1;

/// File name of the catalog's Markdown page.
pub const CATALOG_MARKDOWN: &str = "spu_sequence_relations.md";

/// File name of the catalog's JSON file.
pub const CATALOG_JSON: &str = "spu_sequence_relations.json";

/// The generated catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationCatalog {
    /// The Markdown page.
    pub markdown: String,
    /// The JSON file.
    pub json: String,
}

#[derive(serde::Serialize)]
struct CatalogJson {
    schema_version: u32,
    program_base: String,
    taken_landing: String,
    terminator: String,
    taken_terminator: String,
    components: Vec<&'static str>,
    rows: Vec<RowJson>,
}

#[derive(serde::Serialize)]
struct RowJson {
    name: String,
    registers: &'static [&'static str],
    sequence: Vec<String>,
    partner: PartnerJson,
    class: ClassJson,
    precondition: Option<&'static str>,
    dead: Vec<&'static str>,
    pins: Vec<PinJson>,
    local_store: bool,
    compares: ComparesJson,
    citations: Vec<String>,
    counterexamples: Vec<CounterexampleJson>,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind")]
enum PartnerJson {
    Fused {
        writes: Vec<&'static str>,
        computes: &'static str,
    },
    Guest {
        sequence: Vec<String>,
    },
}

#[derive(serde::Serialize)]
struct ClassJson {
    kind: &'static str,
    ulp: Option<u32>,
}

#[derive(serde::Serialize)]
struct PinJson {
    register: &'static str,
    word: String,
    lanes: &'static str,
}

#[derive(serde::Serialize)]
struct ComparesJson {
    claim: &'static str,
    components: Vec<&'static str>,
    registers_left_out: Vec<&'static str>,
    approximate: Vec<&'static str>,
    ulp: Option<u32>,
}

/// The catalog of every row, with the committed store's counterexamples.
///
/// # Errors
///
/// [`FuzzError`] when the store does not parse or a row does not render.
pub fn relation_catalog() -> Result<RelationCatalog, FuzzError> {
    let stored = stored_counterexamples().map_err(|_| InvariantError::StoredCounterexamples)?;
    render_catalog(sequence_relations(), &stored)
}

/// The catalog of `rows`, each with its counterexamples in `stored`.
pub(crate) fn render_catalog(
    rows: &[SpuSequenceRelation],
    stored: &[RelationCounterexample],
) -> Result<RelationCatalog, FuzzError> {
    let entries = rows
        .iter()
        .map(|row| row_json(row, stored))
        .collect::<Result<Vec<_>, _>>()?;
    let mut markdown = String::from(PREAMBLE);
    for (row, entry) in rows.iter().zip(&entries) {
        row_markdown(&mut markdown, row, entry, stored);
    }
    markdown.push_str(FOOTER);
    let catalog = CatalogJson {
        schema_version: CATALOG_SCHEMA_VERSION,
        program_base: format!("0x{SEQUENCE_PROGRAM_BASE:05x}"),
        taken_landing: format!("0x{SEQUENCE_TAKEN_LANDING:05x}"),
        terminator: format!("0x{TERMINATOR:08x}"),
        taken_terminator: format!("0x{TAKEN_TERMINATOR:08x}"),
        components: COMPONENTS.iter().map(|(_, name)| *name).collect(),
        rows: entries,
    };
    // The catalog's fields are strings, integers, booleans and lists of
    // them, which always serialize.
    let mut json = serde_json::to_string_pretty(&catalog).unwrap_or_default();
    json.push('\n');
    Ok(RelationCatalog { markdown, json })
}

/// The JSON entry of `row`.
fn row_json(
    row: &SpuSequenceRelation,
    stored: &[RelationCounterexample],
) -> Result<RowJson, FuzzError> {
    let unrendered =
        || -> FuzzError { InvariantError::UnencodableSequenceRelation { relation: row.id }.into() };
    let text = row.id.text();
    let names = text.registers;
    let assemble = |words: &[SpuSymbolicWord]| {
        words
            .iter()
            .map(|word| word.assembly(names))
            .collect::<Option<Vec<_>>>()
    };
    let symbolic = |registers: &[u8]| {
        registers
            .iter()
            .map(|&register| names.get(usize::from(register)).copied())
            .collect::<Option<Vec<_>>>()
    };
    let (partner, partner_words) = match (row.partner, text.fused) {
        (SpuSequencePartner::Guest(words), None) => (
            PartnerJson::Guest {
                sequence: assemble(words).ok_or_else(unrendered)?,
            },
            words,
        ),
        (SpuSequencePartner::Fused(fused), Some(computes)) => (
            PartnerJson::Fused {
                writes: symbolic(fused.writes).ok_or_else(unrendered)?,
                computes,
            },
            &[][..],
        ),
        _ => return Err(unrendered()),
    };
    let mut citations: Vec<String> = Vec::new();
    for word in row.sequence.iter().chain(partner_words) {
        let citation = isa_citation(word.kind).ok_or_else(unrendered)?;
        if !citations.contains(&citation) {
            citations.push(citation);
        }
    }
    let (class, ulp) = match row.float_class {
        SpuFloatClass::BitExact => ("BitExact", None),
        SpuFloatClass::BitExactUnderPrecondition => ("BitExactUnderPrecondition", None),
        SpuFloatClass::Inexact { ulp } => ("Inexact", ulp),
    };
    let pins = row
        .pins
        .iter()
        .map(|&(register, pin)| {
            let register = names
                .get(usize::from(register))
                .copied()
                .ok_or_else(unrendered)?;
            Ok(match pin {
                SpuSequencePin::TakenLanding => PinJson {
                    register,
                    word: format!("0x{SEQUENCE_TAKEN_LANDING:08x}"),
                    lanes: "preferred",
                },
                SpuSequencePin::Word(word) => PinJson {
                    register,
                    word: format!("0x{word:08x}"),
                    lanes: "every",
                },
            })
        })
        .collect::<Result<Vec<_>, FuzzError>>()?;
    let dead = symbolic(row.dead).ok_or_else(unrendered)?;
    Ok(RowJson {
        name: format!("{:?}", row.id),
        registers: names,
        sequence: assemble(row.sequence).ok_or_else(unrendered)?,
        partner,
        class: ClassJson { kind: class, ulp },
        precondition: text.precondition,
        dead: dead.clone(),
        pins,
        local_store: row.local_store,
        compares: ComparesJson {
            claim: if dead.is_empty() {
                "equality"
            } else {
                "refinement"
            },
            components: COMPONENTS.iter().map(|(_, name)| *name).collect(),
            registers_left_out: dead,
            approximate: symbolic(row.approximate).ok_or_else(unrendered)?,
            ulp: match row.float_class {
                SpuFloatClass::Inexact { ulp } => ulp,
                _ => Some(0),
            },
        },
        citations,
        counterexamples: stored
            .iter()
            .filter(|counterexample| counterexample.relation == row.id)
            .map(RelationCounterexample::json)
            .collect(),
    })
}

/// A list of names, or `none`.
fn listed(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// Appends the Markdown section of `row`.
fn row_markdown(
    out: &mut String,
    row: &SpuSequenceRelation,
    entry: &RowJson,
    stored: &[RelationCounterexample],
) {
    let _ = writeln!(out, "### {}\n", entry.name);
    let _ = writeln!(out, "- Sequence A: `{}`", entry.sequence.join("; "));
    let _ = match &entry.partner {
        PartnerJson::Guest { sequence } => {
            writeln!(
                out,
                "- Partner: the guest sequence `{}`",
                sequence.join("; ")
            )
        }
        PartnerJson::Fused { writes, computes } => writeln!(
            out,
            "- Partner: a fused form that writes {}: {computes}",
            listed(writes)
        ),
    };
    let class = match row.float_class {
        SpuFloatClass::BitExact => "bit-exact on every start state".to_owned(),
        SpuFloatClass::BitExactUnderPrecondition => {
            "bit-exact on every start state the precondition admits".to_owned()
        }
        SpuFloatClass::Inexact { ulp: Some(ulp) } => {
            format!("inexact, within {ulp} ulp in each lane")
        }
        SpuFloatClass::Inexact { ulp: None } => "inexact, with the distance measured".to_owned(),
    };
    let _ = writeln!(out, "- Class: {class}");
    let _ = writeln!(
        out,
        "- Precondition: {}",
        entry.precondition.unwrap_or("none; every start state")
    );
    let _ = writeln!(out, "- Dead set: {}", listed(&entry.dead));
    let pins: Vec<String> = entry
        .pins
        .iter()
        .map(|pin| {
            let lanes = if pin.lanes == "every" {
                "every word"
            } else {
                "the preferred word"
            };
            format!("{} = {} in {lanes}", pin.register, pin.word)
        })
        .collect();
    let pins: Vec<&str> = pins.iter().map(String::as_str).collect();
    let _ = writeln!(out, "- Pinned: {}", listed(&pins));
    let _ = writeln!(
        out,
        "- Local store: {}",
        if row.local_store {
            "read or written; the start state fills it"
        } else {
            "not used"
        }
    );
    let mut compares = if entry.dead.is_empty() {
        "equality over every observation component".to_owned()
    } else {
        format!(
            "refinement: every observation component, with Registers leaving out {}",
            listed(&entry.dead)
        )
    };
    if !entry.compares.approximate.is_empty() {
        let approximate = listed(&entry.compares.approximate);
        let _ = match entry.compares.ulp {
            Some(ulp) => write!(compares, "; {approximate} lane by lane within {ulp} ulp"),
            None => write!(compares, "; {approximate} measured, not compared"),
        };
    }
    let _ = writeln!(out, "- Compares: {compares}");
    let _ = writeln!(out, "- ISA: {}", entry.citations.join(" "));
    let counterexamples: Vec<&RelationCounterexample> = stored
        .iter()
        .filter(|counterexample| counterexample.relation == row.id)
        .collect();
    if counterexamples.is_empty() {
        let _ = writeln!(out, "- Stored counterexamples: none");
    } else {
        let _ = writeln!(out, "- Stored counterexamples:");
        for counterexample in counterexamples {
            counterexample_markdown(out, entry.registers, counterexample);
        }
    }
    out.push('\n');
}

/// Appends the list item of one stored counterexample.
fn counterexample_markdown(
    out: &mut String,
    names: &[&str],
    counterexample: &RelationCounterexample,
) {
    let assignment: Vec<String> = names
        .iter()
        .zip(&counterexample.assignment)
        .map(|(name, register)| format!("{name}=r{register}"))
        .collect();
    let registers: Vec<String> = counterexample
        .registers
        .iter()
        .map(|(register, value)| format!("r{register}=0x{:032x}", u128::from_be_bytes(*value)))
        .collect();
    let lines: Vec<String> = counterexample
        .local_store
        .iter()
        .map(|(address, value)| format!("0x{address:05x}=0x{:032x}", u128::from_be_bytes(*value)))
        .collect();
    let nonzero = |values: &[String]| {
        if values.is_empty() {
            "all zero".to_owned()
        } else {
            values.join(", ")
        }
    };
    let _ = writeln!(
        out,
        "  - `{}`: {}; start registers: {}; start local store: {}; found diverging in {}",
        counterexample.name,
        assignment.join(", "),
        nonzero(&registers),
        nonzero(&lines),
        component_name(counterexample.first_component)
    );
}

const PREAMBLE: &str = r#"# SPU sequence relations

A sequence relation pairs an SPU instruction sequence, sequence A, with a
partner that must leave the same observed state from the same start state.
The partner is another guest sequence, or a fused form: the operation a
recompiler emits in place of sequence A. This page lists every relation
CellGov checks. `spu_sequence_relations.json` holds the same rows for a
program to read.

## What a row claims

CellGov runs both sides of a row from one start state and compares the
complete observed state after each [Martignoni2012 p:338 s:2]: registers,
local store, program counter, channels, reservation, outcome, effects,
fault discard, FPSCR, signals and interrupts. The fuzz campaigns draw the
start states, and each stored counterexample replays first.

A row that passes is a tested claim, not a proof. Translation validation
proves one translation equal to its source for every input
[Necula2000 p:1 s:1]. A test runs the translation on some inputs and
compares the outputs [Necula2000 p:2 s:1]. A row is a test: it holds on
the start states CellGov ran, and it states nothing about the others.

A row with a dead set claims refinement, not equality. Sequence A leaves
values in the dead registers that nothing later reads, so the partner may
leave any value there [Lopes2021 p:65 s:1]. The comparison leaves those
registers out, except a dead register that shares its real register with a
live register sequence A writes. A row without a dead set claims equality.

## How to read a row

- Registers are symbolic and numbered in order of first appearance, so one
  row covers every register assignment. A name such as `c` or `rt` stands
  for the register the assignment gives it. Two names can share one
  register unless the precondition keeps them apart.
- The program starts at local-store address 0x20000. The branch target
  `taken` is the taken landing at 0x20100. After the program, both sides
  stop at `stop 0x3ffe`; at the taken landing, they stop at `stop 0x3ffd`.
- A pinned register holds a fixed value in every start state.
- The class says how exactly the partner matches. An inexact row compares
  its approximate registers lane by lane within a bound in ULPs, or only
  measures the distance when it has no bound.

## Checking a fused form

`cellgov dev relations-check <FILE>` compares a recompiler's fused form
against sequence A. The file holds result states, not code, so CellGov
runs nothing from it. FILE is JSON:

```json
{
  "schema_version": 1,
  "results": [
    {
      "name": "example",
      "row": "CeqNotEqualFused",
      "assignment": [3, 4, 5, 6],
      "registers": { "4": "00000000000000000000000000000001" },
      "local_store": {},
      "result": {
        "registers": {
          "3": "ffffffffffffffffffffffff00000000",
          "4": "00000000000000000000000000000001",
          "6": "000000000000000000000000ffffffff"
        },
        "local_store": {},
        "flow": "FallThrough"
      }
    }
  ]
}
```

`assignment` gives the real register of each symbolic register.
`registers` and `local_store` list the nonzero registers and 16-byte
local-store lines of the start state, as 32 lowercase hex digits; all
others are zero. `result` gives the complete state the fused form leaves,
in the same form, and `flow` is `FallThrough` or `Taken`. The stored
counterexamples below are start states to use. The command reports each
entry as a match, as a divergence with its first differing component, or
as inapplicable when the start state is outside the precondition. It exits
with status 4 when an entry diverges.

## Rows

"#;

const FOOTER: &str = "---

Generated by `cellgov dev relations-gen`. Do not hand-edit; rerun the
command.
";

#[cfg(test)]
#[path = "tests/catalog_tests.rs"]
mod tests;
