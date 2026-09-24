//! Structured context for syscalls the current model does not answer.
//!
//! `cellgov_lv2_archive::unmodelled_syscalls` joins the run's
//! unsupported syscalls with the committed archive; this module prints
//! the join as one JSON line.

use cellgov_core::Runtime;
use cellgov_lv2_archive::{
    caller_rows, census_class_label, gate_rows, name_rows, parse, takes_caller_evidence,
    unmodelled_syscalls, UnmodelledSyscall, CALLER, CAPABILITY_GATE, NAME,
};
use serde::Serialize;

const NAME_TSV: &str = include_str!("../../../../../docs/lv2/tables/name.tsv");
const GATE_TSV: &str = include_str!("../../../../../docs/lv2/tables/gate.tsv");
const CALLER_TSV: &str = include_str!("../../../../../docs/lv2/tables/caller.tsv");

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
    sites: &'a [u64],
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

impl<'a> From<UnmodelledSyscall<'a>> for ReportRow<'a> {
    fn from(row: UnmodelledSyscall<'a>) -> Self {
        ReportRow {
            ordinal: row.ordinal,
            hits: row.hits,
            names: row
                .names
                .into_iter()
                .map(|name| Name {
                    name: &name.name,
                    source: name.source.label(),
                    reference: name.reference.as_deref(),
                })
                .collect(),
            census: census_class_label(row.census),
            gate: row.gate.map(|gate| Gate {
                state: gate.state.label(),
                reads: gate.reads.as_deref(),
                fail_errno: gate.fail_errno,
            }),
            caller_evidence: row
                .callers
                .into_iter()
                .map(|caller| CallerEvidence {
                    module: &caller.module,
                    sites: &caller.sites,
                })
                .collect(),
        }
    }
}

/// Prints the model's unsupported-syscall inventory enriched with archive data.
///
/// The line is JSON so callers can consume it without parsing console prose.
/// It reads only observability state after the run; this report
/// cannot affect dispatch or a state hash.
pub(super) fn print(rt: &Runtime) {
    let pup = rt
        .lv2_host()
        .firmware_identity()
        .map(|identity| &identity.pup_sha256_bytes);
    // The caller table is the archive's largest; `print` parses it only
    // for a run whose PUP takes caller evidence.
    let callers = if takes_caller_evidence(pup) {
        parse(&CALLER, CALLER_TSV).map(|table| caller_rows(&table))
    } else {
        Ok(Vec::new())
    };
    let (Ok(names), Ok(gates), Ok(callers)) = (
        parse(&NAME, NAME_TSV),
        parse(&CAPABILITY_GATE, GATE_TSV),
        callers,
    ) else {
        debug_assert!(false, "committed LV2 report archives must parse");
        eprintln!("unmodelled_syscall_report_error: committed LV2 archive did not parse");
        return;
    };
    let names = name_rows(&names);
    let gates = gate_rows(&gates);
    let unsupported = rt
        .lv2_host()
        .observability()
        .unsupported_syscalls
        .iter()
        .map(|(&ordinal, witness)| (ordinal, witness.hits));
    let rows: Vec<ReportRow<'_>> = unmodelled_syscalls(unsupported, pup, &names, &gates, &callers)
        .into_iter()
        .map(ReportRow::from)
        .collect();
    if !rows.is_empty() {
        match serde_json::to_string(&rows) {
            Ok(json) => println!("unmodelled_syscall_report: {json}"),
            Err(error) => {
                debug_assert!(
                    false,
                    "unmodelled syscall report serialization failed: {error}"
                );
                eprintln!("unmodelled_syscall_report_error: {error}");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/unmodelled_tests.rs"]
mod tests;
