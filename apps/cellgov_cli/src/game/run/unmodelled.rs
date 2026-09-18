//! Structured context for syscalls the current model does not answer.

use cellgov_core::Runtime;
use cellgov_lv2::archive::{gate_rows, name_rows, parse, GateState, CAPABILITY_GATE, NAME};
use cellgov_ps3_abi::lv2::census::{lookup, PupCensusClass};
use serde::Serialize;

const NAME_TSV: &str = include_str!("../../../../../docs/lv2/name.tsv");
const GATE_TSV: &str = include_str!("../../../../../docs/lv2/gate.tsv");
const CALLER_TSV: &str = include_str!("../../../../../docs/lv2/caller.tsv");

#[derive(Serialize)]
struct Name<'a> {
    name: &'a str,
    source: &'a str,
    reference: Option<&'a str>,
}

#[derive(Serialize)]
struct Gate<'a> {
    state: &'a str,
    reads: Option<&'a str>,
    fail_errno: Option<u32>,
}

#[derive(Serialize)]
struct CallerEvidence<'a> {
    module: &'a str,
    sites: u64,
}

#[derive(Serialize)]
struct ReportRow<'a> {
    ordinal: u64,
    hits: u64,
    names: Vec<Name<'a>>,
    census: &'static str,
    gate: Option<Gate<'a>>,
    caller_evidence: Vec<CallerEvidence<'a>>,
}

fn census_label(class: PupCensusClass) -> &'static str {
    match class {
        PupCensusClass::Implemented => "implemented",
        PupCensusClass::Stub => "stub",
        PupCensusClass::Absent => "absent",
        PupCensusClass::NotExtracted => "not_extracted",
        PupCensusClass::OutOfRange => "out_of_range",
    }
}

fn gate_state_label(state: GateState) -> &'static str {
    match state {
        GateState::Gated => "gated",
        GateState::Ungated => "ungated",
        GateState::NotAnalysed => "not_analysed",
    }
}

fn pup_hex(pup: &[u8; 32]) -> String {
    pup.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn caller_evidence<'a>(pup: &str, ordinal: u64) -> Vec<CallerEvidence<'a>> {
    CALLER_TSV
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut cells = line.split('\t');
            let (Some(row_pup), Some(module), Some(row_ordinal), Some(sites)) =
                (cells.next(), cells.next(), cells.next(), cells.next())
            else {
                return None;
            };
            (row_pup == pup && row_ordinal.parse().ok() == Some(ordinal)).then(|| CallerEvidence {
                module,
                sites: sites
                    .parse()
                    .expect("committed caller archive has integer sites"),
            })
        })
        .collect()
}

/// Prints the model's unsupported-syscall inventory enriched with archive data.
///
/// The line is JSON so callers can consume it without parsing console prose.
/// It reads only observability state after the run; this report
/// cannot affect dispatch or a state hash.
pub(super) fn print(rt: &Runtime) {
    let identity = rt.lv2_host().firmware_identity();
    let pup = identity.map(|value| pup_hex(&value.pup_sha256_bytes));
    let names = parse(&NAME, NAME_TSV).expect("committed name archive parses");
    let gates = parse(&CAPABILITY_GATE, GATE_TSV).expect("committed gate archive parses");
    let names = name_rows(&names);
    let gates = gate_rows(&gates);
    let rows: Vec<ReportRow<'_>> = rt
        .lv2_host()
        .observability()
        .unsupported_syscalls
        .iter()
        .map(|(&ordinal, witness)| {
            let class = identity.map_or(PupCensusClass::NotExtracted, |identity| {
                usize::try_from(ordinal)
                    .ok()
                    .map_or(PupCensusClass::OutOfRange, |value| {
                        lookup(&identity.pup_sha256_bytes, value)
                    })
            });
            let ordinal_index = usize::try_from(ordinal).ok();
            let gate = gates
                .iter()
                .find(|row| {
                    pup.as_deref().is_some_and(|pup| row.pup_sha256 == pup)
                        && Some(row.ordinal) == ordinal_index
                })
                .map(|row| Gate {
                    state: gate_state_label(row.state),
                    reads: row.reads.as_deref(),
                    fail_errno: row.fail_errno,
                });
            ReportRow {
                ordinal,
                hits: witness.hits,
                names: names
                    .iter()
                    .filter(|row| row.ordinal == ordinal && row.packet.is_none())
                    .map(|row| Name {
                        name: &row.name,
                        source: row.source.label(),
                        reference: row.reference.as_deref(),
                    })
                    .collect(),
                census: census_label(class),
                gate,
                caller_evidence: if class == PupCensusClass::NotExtracted {
                    pup.as_deref()
                        .map_or_else(Vec::new, |pup| caller_evidence(pup, ordinal))
                } else {
                    Vec::new()
                },
            }
        })
        .collect();
    if !rows.is_empty() {
        println!(
            "unmodelled_syscall_report: {}",
            serde_json::to_string(&rows).expect("report rows serialize")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{census_label, pup_hex};
    use cellgov_ps3_abi::lv2::census::PupCensusClass;

    #[test]
    fn unknown_census_state_is_reported_not_silently_absent() {
        assert_eq!(census_label(PupCensusClass::NotExtracted), "not_extracted");
    }

    #[test]
    fn pup_digest_uses_the_archive_key_spelling() {
        assert_eq!(pup_hex(&[0xab; 32]), "ab".repeat(32));
    }
}
