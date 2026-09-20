use std::collections::BTreeMap;

use super::LoadedPrx;
use crate::sprx::test_fixtures::make_test_prx_graph_node;

fn loaded_with_ends(text_end: u64, data_end: u64) -> LoadedPrx {
    LoadedPrx {
        name: "test".to_string(),
        module_id: crate::prx_loader::PrxModuleId(1),
        base: 0,
        toc: 0,
        text_start: 0,
        text_end,
        data_start: 0,
        data_end,
        exports: BTreeMap::new(),
        module_start: None,
        module_stop: None,
        relocs_applied: 0,
    }
}

#[test]
fn a_text_tail_above_data_sets_the_resident_end() {
    assert_eq!(
        loaded_with_ends(0x3_0000, 0x2_3700).resident_end(),
        0x3_0000
    );
}

#[test]
fn a_data_tail_above_text_sets_the_resident_end() {
    assert_eq!(
        loaded_with_ends(0x1_0000, 0x1_518c).resident_end(),
        0x1_518c
    );
}

#[test]
fn the_next_module_starts_after_a_text_tail_above_data() {
    let mut modules = BTreeMap::new();
    for (path, module, library) in [
        ("first.sprx", "mod0001", "lib0001"),
        ("second.sprx", "mod0002", "lib0002"),
    ] {
        let mut bytes = make_test_prx_graph_node(module, library, None);
        // PT_LOAD[0] has vaddr 0. Extend its memsz past PT_LOAD[1]'s
        // exclusive end (0x300) so cursor advancement must honor text_end.
        let text_phdr = 64usize;
        bytes[text_phdr + 40..text_phdr + 48].copy_from_slice(&0x1_8000u64.to_be_bytes());
        modules.insert(path.to_string(), bytes);
    }

    let mut memory = cellgov_mem::GuestMemory::new(0x8_0000);
    let image = crate::prx_loader::load_firmware_set(modules, &mut memory, 0x1_0000)
        .expect("two independent modules load");
    assert_eq!(image.topological_order.len(), 2);

    let first = &image.loaded[&image.topological_order[0]];
    let second = &image.loaded[&image.topological_order[1]];
    assert_eq!(first.data_end, 0x1_0300);
    assert_eq!(first.text_end, 0x2_8000);
    assert_eq!(second.base, 0x3_0000);
}
