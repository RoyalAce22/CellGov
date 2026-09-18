//! Structured `dev lv2-discover` report fields.

use super::*;
use cellgov_ppu::lv2_table::{
    Lv2DiscoveryConfidence, Lv2DiscoveryEvidence, Lv2DiscoveryMethod, Lv2TableEntryFormat,
};

#[test]
fn lv2_discover_accepts_the_vfs_root_and_json_globals() {
    let argv: Vec<String> = [
        "cellgov",
        "--vfs-root",
        "store/dev_hdd0",
        "--format",
        "json",
        "dev",
        "lv2-discover",
        "kernel.self",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let cli = crate::cli::parse::try_parse(&argv).expect("the discovery invocation parses");
    assert_eq!(crate::cli::parse::global_refusal(&cli), None);
}

#[test]
fn discovery_document_preserves_method_confidence_and_evidence() {
    let doc = document(
        "kernel.elf",
        Lv2TableDiscovery {
            method: Lv2DiscoveryMethod::ScVectorDescriptorArray,
            confidence: Lv2DiscoveryConfidence::High,
            vector_vaddr: 0x8000_0000_0000_0c00,
            handler_vaddr: 0x8000_0000_0029_7c3c,
            table_vaddr: 0x8000_0000_0034_6570,
            table_file_offset: 0x356570,
            entry_count: 1024,
            entry_width: 8,
            entry_format: Lv2TableEntryFormat::Ppc64DescriptorPointer,
            toc: 0x8000_0000_0033_0540,
            evidence: Lv2DiscoveryEvidence {
                vector_targets: 2,
                handler_matches: 1,
                table_candidates: 1,
                descriptor_entries: 1024,
                unique_descriptors: 637,
                entry_zero_references: 388,
                last_entry_is_entry_zero: true,
                zero_environments: 1024,
                consistent_toc: true,
                post_table_zero: Some(true),
                entry_zero_return: Some(0x8001_0003),
            },
        },
    );
    let value = serde_json::to_value(doc).expect("serialize report");
    assert_eq!(
        value,
        serde_json::json!({
            "format_version": 1,
            "input": "kernel.elf",
            "method": "sc_vector_descriptor_array",
            "confidence": "high",
            "vector_vaddr": "0x8000000000000c00",
            "handler_vaddr": "0x8000000000297c3c",
            "table_vaddr": "0x8000000000346570",
            "table_file_offset": "0x356570",
            "entry_count": 1024,
            "entry_width": 8,
            "entry_format": "ppc64_descriptor_pointer",
            "toc": "0x8000000000330540",
            "evidence": {
                "vector_targets": 2,
                "handler_matches": 1,
                "table_candidates": 1,
                "descriptor_entries": 1024,
                "unique_descriptors": 637,
                "entry_zero_references": 388,
                "last_entry_is_entry_zero": true,
                "zero_environments": 1024,
                "consistent_toc": true,
                "post_table_zero": true,
                "entry_zero_return": "0x80010003"
            }
        })
    );
}
