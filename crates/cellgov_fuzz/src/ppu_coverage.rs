//! The PPU coverage table: one row per reference unit, with each kind
//! of evidence CellGov holds for the unit beside it.
//!
//! The evidence:
//! - what CellGov does with the unit's words, from the decoder and the
//!   fuzz descriptors;
//! - the unit's reference file;
//! - the console-captured probe cases that run one of its words;
//! - the documented open-result sets whose vectors run one of its words;
//! - how many of the installed titles' words it holds, from a census
//!   record.
//!
//! The renderer reads tracked inputs only, so a drift test can
//! regenerate the table on a fresh clone.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use cellgov_ppu::instruction::fuzz::{
    generation_descriptors, PpuFuzzKind, PpuGenerationDescriptor,
};
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::ppu_census::{classify_word, refusal_name, PpuCensusRecord, PpuWordFate};
use crate::ppu_documented::documented_vectors;
use crate::ppu_reference::{
    all_units, decoded_word_unit, file_unit, parse_reference_file, unit_key, word_unit,
    PpuReferenceCampaignError, PpuReferenceUnit,
};

/// What CellGov does with a unit's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuUnitStatus {
    /// Some word of the unit decodes and executes.
    Executes,
    /// Every decoded word of the unit faults, with these classes.
    Refuses(BTreeSet<&'static str>),
    /// No word of the unit decodes.
    NotDecoded,
    /// The unit is a facility, which no single word selects.
    Facility,
}

/// One row of the coverage table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuCoverageRow {
    /// The unit key.
    pub unit: String,
    /// What CellGov does with the unit's words.
    pub status: PpuUnitStatus,
    /// Whether a fuzz generation descriptor covers a kind of the unit.
    pub fuzz_descriptor: bool,
    /// The unit's reference file.
    pub reference_file: Option<String>,
    /// Console-captured probe cases that run a word of the unit.
    pub console_cases: usize,
    /// The documented open-result sets whose vectors run a word of the
    /// unit, by set id.
    pub open_sets: BTreeSet<&'static str>,
    /// The installed titles' words of the unit, or `None` with no census
    /// record.
    pub title_words: Option<u64>,
}

/// The tracked evidence that [`coverage_rows`] reads, beside what the
/// code itself answers.
#[derive(Debug, Clone, Default)]
pub struct PpuCoverageEvidence {
    /// Reference file name by unit key.
    pub reference_files: BTreeMap<String, String>,
    /// Captured console cases by unit key.
    pub console_cases: BTreeMap<String, usize>,
    /// The census of the installed titles.
    pub census: Option<PpuCensusRecord>,
}

/// The reference file of each unit in `dir`, by unit key. The function
/// leaves out a file that does not parse or covers no unit; the
/// reference campaign reports those.
///
/// # Errors
///
/// A directory or file that cannot be read.
pub fn reference_files(dir: &Path) -> Result<BTreeMap<String, String>, PpuReferenceCampaignError> {
    let read_error = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PpuReferenceCampaignError::Read { path, source }
    };
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(read_error(dir))? {
        let name = entry.map_err(read_error(dir))?.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".json") {
            names.push(name);
        }
    }
    names.sort();
    let mut files = BTreeMap::new();
    for name in names {
        let path = dir.join(&name);
        let json = std::fs::read_to_string(&path).map_err(read_error(&path))?;
        let Some(unit) = parse_reference_file(&json)
            .ok()
            .as_ref()
            .and_then(file_unit)
        else {
            continue;
        };
        files.entry(unit_key(&unit)).or_insert(name);
    }
    Ok(files)
}

/// The cases that run a word of each unit, by unit key, from the word
/// lists of captured cases. A case counts once for each unit its words
/// reach.
pub fn console_case_units<'a>(
    cases: impl IntoIterator<Item = &'a [u32]>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for words in cases {
        let units: BTreeSet<String> = words
            .iter()
            .filter_map(|&raw| word_unit(raw))
            .map(|unit| unit_key(&unit))
            .collect();
        for unit in units {
            *counts.entry(unit).or_default() += 1;
        }
    }
    counts
}

/// Every unit's row, in [`all_units`] order.
pub fn coverage_rows(evidence: &PpuCoverageEvidence) -> Vec<PpuCoverageRow> {
    let descriptors = generation_descriptors();
    let status = unit_status(&descriptors);
    let descriptors = descriptor_units(&descriptors);
    let open = open_sets();
    all_units()
        .iter()
        .map(|unit| {
            let fallback = match unit {
                PpuReferenceUnit::Facility { .. } => PpuUnitStatus::Facility,
                _ => PpuUnitStatus::NotDecoded,
            };
            (unit_key(unit), fallback)
        })
        .map(|(unit, fallback)| PpuCoverageRow {
            status: status.get(&unit).cloned().unwrap_or(fallback),
            fuzz_descriptor: descriptors.contains(&unit),
            reference_file: evidence.reference_files.get(&unit).cloned(),
            console_cases: evidence.console_cases.get(&unit).copied().unwrap_or(0),
            open_sets: open.get(&unit).cloned().unwrap_or_default(),
            title_words: evidence
                .census
                .as_ref()
                .map(|census| census.units.get(&unit).map_or(0, |counts| counts.seen())),
            unit,
        })
        .collect()
}

/// The words that stand for the units of `kind`: the canonical word,
/// or for a stream kind the word with bit 6 clear and with bit 6 set,
/// because bit 6 selects a mnemonic of its own.
fn kind_words(kind: PpuFuzzKind, canonical: u32) -> Vec<u32> {
    match kind {
        PpuFuzzKind::Ordinary(
            PpuInstructionKind::Dss | PpuInstructionKind::Dst | PpuInstructionKind::Dstst,
        ) => vec![canonical & !(1 << 25), canonical | (1 << 25)],
        _ => vec![canonical],
    }
}

/// What CellGov does with each decoded unit, by unit key.
fn unit_status(descriptors: &[PpuGenerationDescriptor]) -> BTreeMap<String, PpuUnitStatus> {
    let mut status: BTreeMap<String, PpuUnitStatus> = BTreeMap::new();
    for descriptor in descriptors {
        for raw in kind_words(descriptor.kind, descriptor.canonical_word) {
            let (unit, fate) = classify_word(raw);
            let refusal = match fate {
                PpuWordFate::NotRecognized => continue,
                PpuWordFate::Implemented => None,
                PpuWordFate::Refused(class) => Some(refusal_name(class)),
            };
            let entry = status
                .entry(unit_key(&unit))
                .or_insert_with(|| PpuUnitStatus::Refuses(BTreeSet::new()));
            match (refusal, &mut *entry) {
                (None, _) => *entry = PpuUnitStatus::Executes,
                (Some(class), PpuUnitStatus::Refuses(classes)) => {
                    classes.insert(class);
                }
                (Some(_), _) => {}
            }
        }
    }
    status
}

/// The units a fuzz generation descriptor covers, by unit key.
fn descriptor_units(descriptors: &[PpuGenerationDescriptor]) -> BTreeSet<String> {
    descriptors
        .iter()
        .flat_map(|descriptor| {
            kind_words(descriptor.kind, descriptor.canonical_word)
                .into_iter()
                .map(move |raw| unit_key(&decoded_word_unit(raw, descriptor.kind)))
        })
        .collect()
}

/// The documented open-result sets each unit's words reach, by unit
/// key.
fn open_sets() -> BTreeMap<String, BTreeSet<&'static str>> {
    let mut sets: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    for vector in documented_vectors() {
        for &raw in &vector.words {
            if let Some(unit) = word_unit(raw) {
                sets.entry(unit_key(&unit))
                    .or_default()
                    .insert(vector.set.id());
            }
        }
    }
    sets
}

/// The table as a markdown document.
pub fn render_coverage(rows: &[PpuCoverageRow], census: Option<&PpuCensusRecord>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# PPU coverage\n");
    let _ = writeln!(
        out,
        "Generated by `cellgov dev ppu-coverage-gen`; do not edit by hand. One row per \
         architectural unit of the PPU reference set, with the evidence CellGov holds for it.\n"
    );
    render_columns(&mut out);
    render_summary(&mut out, rows, census);
    render_table(&mut out, rows);
    render_outside(&mut out, census);
    out
}

fn render_columns(out: &mut String) {
    let _ = writeln!(out, "Columns:\n");
    for line in [
        "- **CellGov**: `executes` when some word of the unit decodes and runs; \
         `refuses` and the fault class when every decoded word faults; \
         `not decoded` when no word of the unit decodes; `facility` for a behaviour \
         no single word selects.",
        "- **fuzz**: a fuzz generation descriptor covers the unit; `-` for a facility.",
        "- **reference**: the unit's file under `crates/cellgov_fuzz/tests/fixtures/ppu_reference/`.",
        "- **console**: the console-captured probe cases under `tests/micro/` that run a word of the unit.",
        "- **open results**: the documented sets whose vectors reach a result the documents \
         leave open in the unit, each settled by the `ppu_reference_vectors` capture; \
         `-` when no documented set reaches the unit.",
        "- **title words**: the unit's words in the installed titles' PPU code, from \
         `docs/ppu_census.json`.",
    ] {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out);
}

fn render_summary(out: &mut String, rows: &[PpuCoverageRow], census: Option<&PpuCensusRecord>) {
    let count = |want: &dyn Fn(&PpuCoverageRow) -> bool| rows.iter().filter(|r| want(r)).count();
    let _ = writeln!(out, "## Summary\n");
    let _ = writeln!(out, "| measure | units |");
    let _ = writeln!(out, "| --- | ---: |");
    let lines: [(&str, usize); 6] = [
        ("units", rows.len()),
        ("executed", count(&|r| r.status == PpuUnitStatus::Executes)),
        (
            "refused",
            count(&|r| matches!(r.status, PpuUnitStatus::Refuses(_))),
        ),
        (
            "with a reference file",
            count(&|r| r.reference_file.is_some()),
        ),
        ("with a console case", count(&|r| r.console_cases > 0)),
        (
            "reached by an open-result set",
            count(&|r| !r.open_sets.is_empty()),
        ),
    ];
    for (measure, units) in lines {
        let _ = writeln!(out, "| {measure} | {units} |");
    }
    if let Some(census) = census {
        let exercised = count(&|r| r.title_words.is_some_and(|w| w > 0));
        let unreferenced =
            count(&|r| r.title_words.is_some_and(|w| w > 0) && r.reference_file.is_none());
        let read = census.titles.iter().filter(|t| t.skipped.is_none()).count();
        let words: u64 = census.titles.iter().map(|t| t.words).sum();
        let _ = writeln!(out, "| exercised by the titles | {exercised} |");
        let _ = writeln!(out, "| exercised with no reference file | {unreferenced} |");
        let _ = writeln!(
            out,
            "\nThe census read {read} title(s), {words} PPU word(s)."
        );
    }
    let _ = writeln!(out);
}

fn render_table(out: &mut String, rows: &[PpuCoverageRow]) {
    let _ = writeln!(out, "## Units\n");
    let _ = writeln!(
        out,
        "| unit | CellGov | fuzz | reference | console | open results | title words |"
    );
    let _ = writeln!(out, "| --- | --- | --- | --- | ---: | --- | ---: |");
    for row in rows {
        let status = match &row.status {
            PpuUnitStatus::Executes => "executes".to_owned(),
            PpuUnitStatus::Refuses(classes) => {
                let classes: Vec<&str> = classes.iter().copied().collect();
                format!("refuses: {}", classes.join(", "))
            }
            PpuUnitStatus::NotDecoded => "not decoded".to_owned(),
            PpuUnitStatus::Facility => "facility".to_owned(),
        };
        let fuzz = match (&row.status, row.fuzz_descriptor) {
            (PpuUnitStatus::Facility, _) => "-",
            (_, true) => "yes",
            (_, false) => "no",
        };
        let open: Vec<&str> = row.open_sets.iter().copied().collect();
        let _ = writeln!(
            out,
            "| `{}` | {status} | {fuzz} | {} | {} | {} | {} |",
            row.unit,
            row.reference_file
                .as_deref()
                .map_or("-".to_owned(), |f| format!("`{f}`")),
            row.console_cases,
            if open.is_empty() {
                "-".to_owned()
            } else {
                open.join(", ")
            },
            row.title_words.map_or("-".to_owned(), |w| w.to_string()),
        );
    }
    let _ = writeln!(out);
}

/// The census units outside the unit list: mnemonics the decoder names
/// in a rejection that no documented listing holds.
fn render_outside(out: &mut String, census: Option<&PpuCensusRecord>) {
    let Some(census) = census else {
        return;
    };
    let units: BTreeSet<String> = all_units().iter().map(unit_key).collect();
    let outside: Vec<(&String, u64)> = census
        .units
        .iter()
        .filter(|(unit, _)| !units.contains(*unit))
        .map(|(unit, counts)| (unit, counts.seen()))
        .collect();
    if outside.is_empty() {
        return;
    }
    let _ = writeln!(out, "## Census words outside the units\n");
    let _ = writeln!(out, "| unit | title words |");
    let _ = writeln!(out, "| --- | ---: |");
    for (unit, words) in outside {
        let _ = writeln!(out, "| `{unit}` | {words} |");
    }
    let _ = writeln!(out);
}

#[cfg(test)]
#[path = "tests/ppu_coverage_tests.rs"]
mod tests;
