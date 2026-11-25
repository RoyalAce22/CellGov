use super::*;

const BASE: u64 = page_table::PAGE_TABLE_STORAGE_BASE;

fn space(htab_size_exp: u64) -> AddressSpace {
    AddressSpace {
        vas_id: 2,
        htab_size_exp,
        page_size_count: KERNEL_PAGE_SIZE_COUNT,
        page_shifts: KERNEL_PAGE_SHIFTS,
        mapping: None,
    }
}

/// A state that kept a space of exponent `first` and then a space of
/// exponent `second`, both with vas id 2.
fn two_with_one_vas_id(first: u64, second: u64) -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(space(first)));
    spaces.apply(AddressSpaceChange::Constructed(space(second)));
    spaces
}

#[test]
fn two_spaces_with_one_vas_id_hash_two_distinct_page_table_addresses() {
    let spaces = two_with_one_vas_id(HTAB_SIZE_EXP_MIN, HTAB_SIZE_EXP_MIN);
    let second_table = BASE + htab_size(HTAB_SIZE_EXP_MIN);
    let expected = [
        lanes::value_term(ADDRESS_SPACE_SOURCE, 0, &space(HTAB_SIZE_EXP_MIN)),
        lanes::value_term(PAGE_TABLE_STORAGE_SOURCE, 0, &BASE),
        lanes::value_term(ADDRESS_SPACE_SOURCE, 1, &space(HTAB_SIZE_EXP_MIN)),
        lanes::value_term(PAGE_TABLE_STORAGE_SOURCE, 1, &second_table),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(spaces.sync_term(), expected);
}

#[test]
fn the_next_table_follows_each_space_own_table_under_one_vas_id() {
    let spaces = two_with_one_vas_id(HTAB_SIZE_EXP_MIN + 1, HTAB_SIZE_EXP_MIN);
    let second_table = BASE + htab_size(HTAB_SIZE_EXP_MIN + 1);
    assert_eq!(
        spaces.next_table(HTAB_SIZE_EXP_MIN),
        Some(second_table + htab_size(HTAB_SIZE_EXP_MIN))
    );
}

#[test]
fn the_table_of_a_shared_vas_id_is_the_first_construction_table() {
    let spaces = two_with_one_vas_id(HTAB_SIZE_EXP_MIN, HTAB_SIZE_EXP_MIN);
    assert_eq!(spaces.table_real(2), Some(BASE));
}
