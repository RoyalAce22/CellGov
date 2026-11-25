use super::*;

fn key(lpar: u64, n1: u64, n2: u64, n3: u64, n4: u64) -> RepositoryKey {
    RepositoryKey {
        lpar,
        n1,
        n2,
        n3,
        n4,
    }
}

#[test]
fn a_path_drops_the_trailing_zero_words() {
    let pun = key(2, first_field(b"bi", 0), field(b"pun", 0), 0, 0);
    assert_eq!(pun.path(), "bi.pun");
    assert_eq!(pun.to_string(), "lpar 2 bi.pun");
}

#[test]
fn a_path_keeps_the_index_of_a_named_word() {
    assert_eq!(
        key(2, first_field(b"bi", 0), field(b"pu", 1), 0, 0).path(),
        "bi.pu+1"
    );
    assert_eq!(
        key(1, first_field(b"bus", 3), field(b"type", 0), 0, 0).path(),
        "bus+3.type"
    );
}

#[test]
fn a_path_renders_a_numeric_word_in_hex_and_keeps_an_inner_zero_word() {
    assert_eq!(
        key(
            2,
            first_field(b"bi", 0),
            field(b"pu", 0),
            1,
            field(b"rm_addr", 0)
        )
        .path(),
        "bi.pu.0x1.rm_addr"
    );
    assert_eq!(
        key(1, first_field(b"be", 0), 0, field(b"clock", 0), 0).path(),
        "be.0.clock"
    );
}

#[test]
fn bi_pu_index_1_and_bi_pu_with_id_1_render_apart() {
    let indexed = key(2, first_field(b"bi", 0), field(b"pu", 1), 0, 0);
    let with_id = key(2, first_field(b"bi", 0), field(b"pu", 0), 1, 0);
    assert_ne!(indexed, with_id);
    assert_eq!(indexed.path(), "bi.pu+1");
    assert_eq!(with_id.path(), "bi.pu.0x1");
}

#[test]
fn a_first_word_with_a_vendor_id_renders_in_hex() {
    assert_eq!(key(1, 0x0000_0001_6269_0000, 0, 0, 0).path(), "0x162690000");
}

#[test]
fn the_name_0x1_and_the_number_1_render_alike_as_different_words() {
    let name = key(2, first_field(b"bi", 0), field(b"0x1", 0), 0, 0);
    let number = key(2, first_field(b"bi", 0), 1, 0, 0);
    assert_ne!(name, number);
    assert_eq!(name.path(), "bi.0x1");
    assert_eq!(number.path(), "bi.0x1");
}

#[test]
fn a_word_whose_high_byte_is_not_a_name_byte_renders_in_hex() {
    assert_eq!(
        key(2, first_field(b"bi", 0), u64::from(b'b'), 0, 0).path(),
        "bi.0x62"
    );
}

#[test]
fn a_full_8_byte_name_renders_without_an_index() {
    assert_eq!(
        key(2, first_field(b"bi", 0), field(b"rgntotal", 0), 0, 0).path(),
        "bi.rgntotal"
    );
}
