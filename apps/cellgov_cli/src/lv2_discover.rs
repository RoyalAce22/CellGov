//! Loads an LV2 kernel and reports its syscall dispatch-table discovery.

use cellgov_ppu::lv2_stub::{self, Lv2StubClassification, Lv2StubClassificationError};
use cellgov_ppu::lv2_table::{self, Lv2TableDiscovery};

use crate::cli::exit::{decrypt_ppu_self_or_die, die, load_file_or_die};
use crate::cli::parse::{Lv2DiscoverArgs, OutputFormat};
use crate::cli::title::resolve_ps3_vfs_root;

#[derive(Debug, serde::Serialize)]
struct Lv2DiscoverDoc {
    format_version: u32,
    input: String,
    method: &'static str,
    confidence: &'static str,
    vector_vaddr: String,
    handler_vaddr: String,
    table_vaddr: String,
    table_file_offset: String,
    entry_count: usize,
    entry_width: usize,
    entry_format: &'static str,
    toc: String,
    evidence: Lv2DiscoverEvidenceDoc,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification: Option<Lv2ClassificationDoc>,
}

#[derive(Debug, serde::Serialize)]
struct Lv2DiscoverEvidenceDoc {
    vector_targets: usize,
    handler_matches: usize,
    table_candidates: usize,
    descriptor_entries: usize,
    unique_descriptors: usize,
    entry_zero_references: usize,
    last_entry_is_entry_zero: bool,
    zero_environments: usize,
    consistent_toc: bool,
    post_table_zero: Option<bool>,
    entry_zero_return: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Lv2ClassificationDoc {
    Classified {
        implemented: usize,
        stub: usize,
        absent: usize,
        primary_stub: Lv2StubTargetDoc,
        stub_targets: Vec<Lv2StubTargetDoc>,
        evidence: Lv2StubEvidenceDoc,
        ordinals: Vec<Lv2OrdinalDoc>,
    },
    Refused {
        reason: String,
    },
}

#[derive(Debug, serde::Serialize)]
struct Lv2StubTargetDoc {
    descriptor: String,
    code: String,
    errno: String,
    errno_symbol: &'static str,
    references: usize,
}

#[derive(Debug, serde::Serialize)]
struct Lv2StubEvidenceDoc {
    descriptor_targets: usize,
    mode_references: usize,
    runner_up_references: usize,
    minimum_mode_references: usize,
    minimum_dominance_factor: usize,
}

#[derive(Debug, serde::Serialize)]
struct Lv2OrdinalDoc {
    ordinal: usize,
    class: &'static str,
    descriptor: Option<String>,
    code: Option<String>,
}

pub(crate) fn run(
    args: &Lv2DiscoverArgs,
    vfs_flag: Option<&std::path::Path>,
    format: OutputFormat,
) {
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let raw = load_file_or_die(&args.path);
    let elf = decrypt_ppu_self_or_die(&raw, &args.path, &vfs_root);
    let discovery = lv2_table::discover(&elf)
        .unwrap_or_else(|error| die(&format!("lv2-discover: {}: {error}", args.path)));
    let classification = match lv2_stub::classify_discovered(&elf, discovery) {
        Ok(classification) => classification_document(&classification),
        Err(error) => refusal_document(error),
    };
    let doc = document(&args.path, discovery, Some(classification));
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&doc).expect("discovery report is plain data")
        ),
        OutputFormat::Human => render_human(&doc),
    }
}

fn document(
    input: &str,
    discovery: Lv2TableDiscovery,
    classification: Option<Lv2ClassificationDoc>,
) -> Lv2DiscoverDoc {
    Lv2DiscoverDoc {
        format_version: if classification.is_some() { 2 } else { 1 },
        input: input.to_string(),
        method: discovery.method.as_str(),
        confidence: discovery.confidence.as_str(),
        vector_vaddr: hex(discovery.vector_vaddr),
        handler_vaddr: hex(discovery.handler_vaddr),
        table_vaddr: hex(discovery.table_vaddr),
        table_file_offset: format!("0x{:x}", discovery.table_file_offset),
        entry_count: discovery.entry_count,
        entry_width: discovery.entry_width,
        entry_format: discovery.entry_format.as_str(),
        toc: hex(discovery.toc),
        evidence: Lv2DiscoverEvidenceDoc {
            vector_targets: discovery.evidence.vector_targets,
            handler_matches: discovery.evidence.handler_matches,
            table_candidates: discovery.evidence.table_candidates,
            descriptor_entries: discovery.evidence.descriptor_entries,
            unique_descriptors: discovery.evidence.unique_descriptors,
            entry_zero_references: discovery.evidence.entry_zero_references,
            last_entry_is_entry_zero: discovery.evidence.last_entry_is_entry_zero,
            zero_environments: discovery.evidence.zero_environments,
            consistent_toc: discovery.evidence.consistent_toc,
            post_table_zero: discovery.evidence.post_table_zero,
            entry_zero_return: discovery.evidence.entry_zero_return.map(hex32),
        },
        classification,
    }
}

fn classification_document(classification: &Lv2StubClassification) -> Lv2ClassificationDoc {
    Lv2ClassificationDoc::Classified {
        implemented: classification.implemented,
        stub: classification.stub,
        absent: classification.absent,
        primary_stub: stub_target_document(&classification.primary_stub),
        stub_targets: classification
            .stub_targets
            .iter()
            .map(stub_target_document)
            .collect(),
        evidence: Lv2StubEvidenceDoc {
            descriptor_targets: classification.evidence.descriptor_targets,
            mode_references: classification.evidence.mode_references,
            runner_up_references: classification.evidence.runner_up_references,
            minimum_mode_references: classification.evidence.minimum_mode_references,
            minimum_dominance_factor: classification.evidence.minimum_dominance_factor,
        },
        ordinals: classification
            .ordinals
            .iter()
            .map(|entry| Lv2OrdinalDoc {
                ordinal: entry.ordinal,
                class: entry.class.as_str(),
                descriptor: entry.descriptor.map(hex),
                code: entry.code.map(hex),
            })
            .collect(),
    }
}

fn refusal_document(error: Lv2StubClassificationError) -> Lv2ClassificationDoc {
    Lv2ClassificationDoc::Refused {
        reason: error.to_string(),
    }
}

fn stub_target_document(stub: &cellgov_ppu::lv2_stub::Lv2StubTarget) -> Lv2StubTargetDoc {
    Lv2StubTargetDoc {
        descriptor: hex(stub.descriptor),
        code: hex(stub.code),
        errno: hex32(stub.errno),
        errno_symbol: stub.errno_symbol,
        references: stub.references,
    }
}

fn render_human(doc: &Lv2DiscoverDoc) {
    println!("LV2 dispatch table: {}", doc.input);
    println!("  method {} confidence {}", doc.method, doc.confidence);
    println!(
        "  vector {} handler {}",
        doc.vector_vaddr, doc.handler_vaddr
    );
    println!(
        "  table {} file {} entries {} width {} format {}",
        doc.table_vaddr, doc.table_file_offset, doc.entry_count, doc.entry_width, doc.entry_format
    );
    println!("  toc {}", doc.toc);
    println!(
        "  evidence: vector_targets={} handler_matches={} table_candidates={} \
         descriptors={}/{} unique={} entry0_refs={} last_is_entry0={} zero_env={} consistent_toc={} post_table_zero={} entry0_return={}",
        doc.evidence.vector_targets,
        doc.evidence.handler_matches,
        doc.evidence.table_candidates,
        doc.evidence.descriptor_entries,
        doc.entry_count,
        doc.evidence.unique_descriptors,
        doc.evidence.entry_zero_references,
        doc.evidence.last_entry_is_entry_zero,
        doc.evidence.zero_environments,
        doc.evidence.consistent_toc,
        doc.evidence
            .post_table_zero
            .map(|value| value.to_string())
            .as_deref()
            .unwrap_or("unknown"),
        doc.evidence.entry_zero_return.as_deref().unwrap_or("unknown"),
    );
    match &doc.classification {
        Some(Lv2ClassificationDoc::Classified {
            implemented,
            stub,
            absent,
            primary_stub,
            stub_targets,
            evidence,
            ..
        }) => {
            println!(
                "  classification: implemented={} stub={} absent={} mode_refs={} runner_up_refs={}",
                implemented, stub, absent, evidence.mode_references, evidence.runner_up_references,
            );
            println!(
                "  primary stub: descriptor={} code={} errno={} ({}) references={}",
                primary_stub.descriptor,
                primary_stub.code,
                primary_stub.errno,
                primary_stub.errno_symbol,
                primary_stub.references,
            );
            for candidate in stub_targets {
                if candidate.descriptor == primary_stub.descriptor {
                    continue;
                }
                println!(
                    "  additional stub: descriptor={} code={} errno={} ({}) references={}",
                    candidate.descriptor,
                    candidate.code,
                    candidate.errno,
                    candidate.errno_symbol,
                    candidate.references,
                );
            }
        }
        Some(Lv2ClassificationDoc::Refused { reason }) => {
            println!("  classification: refused ({reason})");
        }
        None => {}
    }
}

fn hex(value: u64) -> String {
    format!("0x{value:016x}")
}

fn hex32(value: u32) -> String {
    format!("0x{value:08x}")
}

#[cfg(test)]
#[path = "tests/lv2_discover_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/lv2_stub_tests.rs"]
mod stub_tests;
