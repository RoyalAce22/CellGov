use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Register,
    Immediate,
    Flag,
}

impl OperandClass for Class {
    const REGISTER: Self = Class::Register;
    const IMMEDIATE: Self = Class::Immediate;
}

fn field(class: Class, mask: u32) -> OperandField<Class> {
    OperandField { class, mask }
}

#[test]
fn low_mask_saturates_at_the_word_width() {
    assert_eq!(low_mask(0), 0);
    assert_eq!(low_mask(5), 0x1f);
    assert_eq!(low_mask(31), 0x7fff_ffff);
    assert_eq!(low_mask(32), u32::MAX);
    assert_eq!(low_mask(40), u32::MAX);
}

#[test]
fn a_split_field_packs_and_spreads_in_bit_order() {
    let mask = 0b1100_0011;

    assert_eq!(extract_bits(0b1000_0001, mask), 0b1001);
    assert_eq!(deposit_bits(0b1001, mask), 0b1000_0001);
    assert_eq!(deposit_bits(0xffff_fff0, mask), 0);
    for value in 0..16 {
        assert_eq!(extract_bits(deposit_bits(value, mask), mask), value);
    }
}

#[test]
fn boundary_values_cover_zero_one_the_sign_edge_and_the_maximum() {
    assert_eq!(
        field(Class::Immediate, 0xff).boundary_values(),
        [0, 1, 0x7f, 0x80, 0xff]
    );
    assert_eq!(field(Class::Flag, 0x10).boundary_values(), [0, 1]);
    assert_eq!(field(Class::Immediate, u32::MAX).maximum(), u32::MAX);
}

#[test]
fn packing_checks_the_count_before_the_range() {
    let operands = [
        field(Class::Register, 0x1f),
        field(Class::Immediate, 0xff00),
    ];

    assert_eq!(
        pack_operands(0, &operands, &[99]),
        Err(OperandPackError::Count {
            expected: 2,
            found: 1
        })
    );
    assert_eq!(
        pack_operands(0, &operands, &[32, 0]),
        Err(OperandPackError::OutOfRange)
    );
    assert_eq!(
        pack_operands(0xffff_ffff, &operands, &[3, 0x12]),
        Ok(0xffff_12e3)
    );
}

#[test]
fn canonical_parameters_read_each_field_of_the_word() {
    let operands = [
        field(Class::Register, 0x1f),
        field(Class::Immediate, 0xff00),
    ];

    assert_eq!(canonical_parameters(0x0000_3405, &operands), [5, 0x34]);
}

#[test]
fn boundary_words_vary_only_immediates_and_keep_what_encode_accepts() {
    let operands = [field(Class::Register, 0x1f), field(Class::Immediate, 0x300)];
    let mut seen = Vec::new();

    let words = immediate_boundary_words(0x0000_0107, &operands, |parameters| {
        seen.push(parameters.to_vec());
        (parameters[1] != 2).then(|| parameters[0] | parameters[1] << 8)
    });

    assert_eq!(seen, [[7, 0], [7, 1], [7, 2], [7, 3]]);
    assert_eq!(words, [0x007, 0x107, 0x307]);
}

#[test]
fn register_aliasing_sets_every_register_field_and_nothing_else() {
    let operands = [
        field(Class::Register, 0x1f),
        field(Class::Immediate, 0xff00),
        field(Class::Register, 0x7_0000),
    ];

    assert_eq!(
        register_alias_parameters(0x0000_1200, &operands, 0x1d),
        Some(vec![0x1d, 0x12, 0x5])
    );
    assert_eq!(
        register_alias_parameters(0, &[field(Class::Immediate, 0xff)], 3),
        None
    );
}

#[test]
fn operand_bit_clears_walk_fields_then_bits_and_skip_clear_and_foreign_bits() {
    let operands = [field(Class::Register, 0xf0), field(Class::Immediate, 0x3)];

    assert_eq!(operand_mask(&operands), 0xf3);
    assert_eq!(
        operand_bit_clears(0x1_0f53, &operands),
        [0x1_0f43, 0x1_0f13, 0x1_0f52, 0x1_0f51]
    );
}
