//! Loader and selection refusals for relocations the applier cannot
//! address.

use std::collections::BTreeMap;

use crate::prx_loader::{check_loadable, select_import_closure, PruneReason, PrxLoaderError};
use crate::sprx::test_fixtures::make_test_prx_graph_node;

/// The `sym` word of the fixture's first RELA entry.
const FIRST_RELOC_SYM: std::ops::Range<usize> = 0x3F8..0x3FC;

fn with_first_reloc_sym(module: &str, lib: &str, sym: u32) -> Vec<u8> {
    let mut bytes = make_test_prx_graph_node(module, lib, None);
    // Pins the constant to the fixture: the first entry relocates text
    // against text, so its sym word reads zero before the patch.
    assert_eq!(
        u32::from_be_bytes(bytes[FIRST_RELOC_SYM].try_into().expect("4-byte sym")),
        0,
        "FIRST_RELOC_SYM no longer names the first RELA entry's sym word",
    );
    bytes[FIRST_RELOC_SYM].copy_from_slice(&sym.to_be_bytes());
    bytes
}

/// Builds a module with `segment_count` PT_LOADs and one relocation.
/// Only `text_idx` and `data_idx` carry content.
fn parsed_with_segments(
    segment_count: usize,
    text_idx: usize,
    data_idx: usize,
    sym: u32,
) -> crate::sprx::ParsedPrx {
    let seg = |index: usize| crate::sprx::PrxSegment {
        index,
        vaddr: 0x1000 * index as u64,
        filesz: 0x100,
        memsz: 0x100,
        data: vec![0u8; 0x100],
    };
    crate::sprx::ParsedPrx {
        name: "padded".to_string(),
        module_id: crate::prx_loader::graph::module_id_from_name("padded"),
        toc: 0,
        text: seg(text_idx),
        data: seg(data_idx),
        segment_vaddrs: (0..segment_count).map(|i| 0x1000 * i as u64).collect(),
        exports: Vec::new(),
        relocations: vec![crate::sprx::PrxRelocation {
            offset: 0,
            rtype: 1,
            sym,
            addend: 0,
        }],
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn a_relocation_naming_no_value_segment_is_refused() {
    // sym 0xFF00: target segment 0, value segment 0xFF.
    let bytes = with_first_reloc_sym("modffff", "libffff", 0xFF00);
    let err = check_loadable(&bytes).unwrap_err();
    assert!(
        matches!(
            err,
            PrxLoaderError::RelocWithoutValueSegment { rtype: 1, .. }
        ),
        "expected a no-value-segment refusal, got {err:?}",
    );
}

#[test]
fn a_relocation_past_the_declared_segments_is_refused() {
    // sym 0x0203: target segment 3, value segment 2; the module
    // declares two PT_LOADs.
    let bytes = with_first_reloc_sym("modeeee", "libeeee", 0x0203);
    let err = check_loadable(&bytes).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::RelocSegmentOutOfRange {
            module: crate::prx_loader::graph::module_id_from_name("modeeee"),
            segment_idx: 3,
            segment_count: 2,
        }
    );
}

#[test]
fn a_relocation_patching_into_a_placeholder_segment_is_refused() {
    // Text at 0, data at 3, placeholders at 1, 2 and 4. The entry
    // patches into placeholder 1: an index in range that holds no
    // bytes.
    let parsed = parsed_with_segments(5, 0, 3, 0x0001);
    let err = super::check_relocations_addressable(&parsed).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::RelocTargetSegmentEmpty {
            module: crate::prx_loader::graph::module_id_from_name("padded"),
            segment_idx: 1,
        }
    );
}

#[test]
fn a_relocation_resolving_against_a_placeholder_segment_is_refused() {
    // sym 0x0200: target segment 0, value segment 2. Placeholder 2 is
    // in range and declares a vaddr, but the loader allocates nothing
    // there.
    let parsed = parsed_with_segments(5, 0, 3, 0x0200);
    let err = super::check_relocations_addressable(&parsed).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::RelocValueSegmentEmpty {
            module: crate::prx_loader::graph::module_id_from_name("padded"),
            segment_idx: 2,
        }
    );
}

#[test]
fn a_relocation_into_the_data_segment_past_a_placeholder_is_accepted() {
    let parsed = parsed_with_segments(5, 0, 3, 0x0303);
    assert!(super::check_relocations_addressable(&parsed).is_ok());
}

#[test]
fn selection_prunes_each_shape_under_its_own_reason() {
    let candidates: BTreeMap<String, Vec<u8>> = [
        (
            "keep.sprx".to_string(),
            make_test_prx_graph_node("modaaaa", "libaaaa", None),
        ),
        (
            "absolute.sprx".to_string(),
            with_first_reloc_sym("modffff", "libffff", 0xFF00),
        ),
        (
            "past-end.sprx".to_string(),
            with_first_reloc_sym("modeeee", "libeeee", 0x0203),
        ),
    ]
    .into_iter()
    .collect();

    let selection = select_import_closure(&candidates, None).expect("select");
    assert_eq!(
        selection.selected.iter().collect::<Vec<_>>(),
        vec!["keep.sprx"]
    );
    assert_eq!(
        selection.pruned,
        vec![
            (
                "absolute.sprx".to_string(),
                PruneReason::RelocWithoutValueSegment { rtype: 1 }
            ),
            (
                "past-end.sprx".to_string(),
                PruneReason::RelocSegmentOutOfRange {
                    segment_idx: 3,
                    segment_count: 2,
                }
            ),
        ]
    );
}
