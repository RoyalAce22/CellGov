//! Loads an LV2 kernel and reports its syscall dispatch-table discovery.

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
    let doc = document(&args.path, discovery);
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&doc).expect("discovery report is plain data")
        ),
        OutputFormat::Human => render_human(&doc),
    }
}

fn document(input: &str, discovery: Lv2TableDiscovery) -> Lv2DiscoverDoc {
    Lv2DiscoverDoc {
        format_version: 1,
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
