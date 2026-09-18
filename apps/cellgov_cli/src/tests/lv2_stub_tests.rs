use super::*;
use cellgov_ppu::lv2_stub::{
    Lv2ClassifiedOrdinal, Lv2OrdinalClass, Lv2StubEvidence, Lv2StubTarget,
};
use cellgov_ppu::lv2_table::{
    Lv2DiscoveryConfidence, Lv2DiscoveryEvidence, Lv2DiscoveryMethod, Lv2TableEntryFormat,
};

fn sample_discovery() -> Lv2TableDiscovery {
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
    }
}

#[test]
fn classified_document_records_stub_errno_and_each_ordinal() {
    let primary_stub = Lv2StubTarget {
        descriptor: 0x8000_0000_0032_4968,
        code: 0x8000_0000_0029_04b0,
        errno: 0x8001_0003,
        errno_symbol: "CELL_ENOSYS",
        references: 388,
    };
    let classification = Lv2StubClassification {
        discovery: sample_discovery(),
        primary_stub,
        stub_targets: vec![primary_stub],
        ordinals: vec![
            Lv2ClassifiedOrdinal {
                ordinal: 0,
                class: Lv2OrdinalClass::Stub,
                descriptor: Some(primary_stub.descriptor),
                code: Some(primary_stub.code),
            },
            Lv2ClassifiedOrdinal {
                ordinal: 1,
                class: Lv2OrdinalClass::Implemented,
                descriptor: Some(0x8000_0000_0032_4980),
                code: Some(0x8000_0000_0029_0500),
            },
            Lv2ClassifiedOrdinal {
                ordinal: 2,
                class: Lv2OrdinalClass::Absent,
                descriptor: None,
                code: None,
            },
        ],
        implemented: 1,
        stub: 1,
        absent: 1,
        evidence: Lv2StubEvidence {
            descriptor_targets: 637,
            mode_references: 388,
            runner_up_references: 1,
            minimum_mode_references: 256,
            minimum_dominance_factor: 4,
        },
    };
    let value = serde_json::to_value(document(
        "kernel.elf",
        classification.discovery,
        Some(classification_document(&classification)),
    ))
    .expect("serialize classified report");
    assert_eq!(value["format_version"], 2);
    assert_eq!(
        value["classification"],
        serde_json::json!({
            "status": "classified",
            "implemented": 1,
            "stub": 1,
            "absent": 1,
            "primary_stub": {
                "descriptor": "0x8000000000324968",
                "code": "0x80000000002904b0",
                "errno": "0x80010003",
                "errno_symbol": "CELL_ENOSYS",
                "references": 388
            },
            "stub_targets": [{
                "descriptor": "0x8000000000324968",
                "code": "0x80000000002904b0",
                "errno": "0x80010003",
                "errno_symbol": "CELL_ENOSYS",
                "references": 388
            }],
            "evidence": {
                "descriptor_targets": 637,
                "mode_references": 388,
                "runner_up_references": 1,
                "minimum_mode_references": 256,
                "minimum_dominance_factor": 4
            },
            "ordinals": [
                {
                    "ordinal": 0,
                    "class": "stub",
                    "descriptor": "0x8000000000324968",
                    "code": "0x80000000002904b0"
                },
                {
                    "ordinal": 1,
                    "class": "implemented",
                    "descriptor": "0x8000000000324980",
                    "code": "0x8000000000290500"
                },
                {
                    "ordinal": 2,
                    "class": "absent",
                    "descriptor": null,
                    "code": null
                }
            ]
        })
    );
}

#[test]
fn refused_classification_keeps_the_discovery_and_reason() {
    let value = serde_json::to_value(document(
        "kernel.elf",
        sample_discovery(),
        Some(refusal_document(Lv2StubClassificationError::NoClearMode {
            top: 4,
            runner_up: 4,
            minimum: 4,
            factor: 4,
        })),
    ))
    .expect("serialize refusal report");
    assert_eq!(
        value,
        serde_json::json!({
            "format_version": 2,
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
            },
            "classification": {
                "status": "refused",
                "reason": "LV2 stub classification: no clear mode (top=4, runner_up=4, minimum=4, factor=4)"
            }
        })
    );
}
