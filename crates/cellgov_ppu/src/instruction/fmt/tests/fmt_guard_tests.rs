//! Guards over every generation descriptor: each decodable kind's text
//! names its own mnemonic, and each operand bit the decoder reads
//! changes the text.

use cellgov_exec::operand::canonical_parameters;

use super::simplify::simplify;
use crate::decode::decode;
use crate::instruction::fuzz::{
    generation_descriptors, PpuFuzzKind, PpuGenerationDescriptor, PpuOperandClass,
};
use crate::instruction::{
    ppe_illegal_mnemonic, ppe_privileged_mnemonic, AsmText, PpuInstructionKind,
};

fn text(raw: u32) -> String {
    let insn = decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e}"));
    AsmText {
        insn: &insn,
        addr: 0x1_0000,
        symbols: None,
    }
    .to_string()
}

/// The mnemonic the text of `raw` must start with, before any `.`.
fn expected_mnemonic(kind: PpuFuzzKind, raw: u32) -> String {
    let name: &'static str = match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::PpeIllegal) => {
            ppe_illegal_mnemonic(raw).unwrap_or("<none>")
        }
        PpuFuzzKind::Ordinary(PpuInstructionKind::PpePrivileged) => {
            ppe_privileged_mnemonic(raw).unwrap_or("<none>")
        }
        PpuFuzzKind::Ordinary(kind) => kind.into(),
        PpuFuzzKind::Vx(op) => op.into(),
        PpuFuzzKind::Va(op) => op.into(),
        PpuFuzzKind::Fp59(op) => op.into(),
        PpuFuzzKind::Fp63(op) => op.into(),
    };
    // A record-form kind (`AddicDot`) spells its dot as a suffix.
    let name = name.to_lowercase();
    name.strip_suffix("dot").map_or(name.clone(), str::to_owned)
}

#[test]
fn every_descriptor_text_names_its_own_mnemonic() {
    let mut wrong = Vec::new();
    let mut checked = 0;
    for d in &generation_descriptors() {
        let insn = decode(d.canonical_word).expect("a canonical word decodes");
        // An extended mnemonic (`li`, `mr`, `blr`) replaces the base one.
        if simplify(&insn, 0x1_0000).is_some() {
            continue;
        }
        checked += 1;
        let shown = text(d.canonical_word);
        let want = expected_mnemonic(d.kind, d.canonical_word);
        let first = shown.split_whitespace().next().unwrap_or("");
        if first.trim_end_matches('.') != want {
            wrong.push(format!("{:?}: {shown:?}, want {want}", d.kind));
        }
    }
    assert!(checked > 300, "{checked} kinds checked");
    assert!(
        wrong.is_empty(),
        "{} kinds:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A branch-conditional BO field, whose at and z hint bits the text drops.
///
/// [PPC-Book1 p:20 s:2.4.1] Figure 21 marks the z, a and t bits of BO.
fn is_hinted_bo(d: &PpuGenerationDescriptor, mask: u32, class: PpuOperandClass) -> bool {
    matches!(
        d.kind,
        PpuFuzzKind::Ordinary(
            PpuInstructionKind::Bc | PpuInstructionKind::Bclr | PpuInstructionKind::Bcctr
        )
    ) && mask == 0x03e0_0000
        && class == PpuOperandClass::Condition
}

/// For each bit of field `index`, the word with that bit flipped, when
/// `encode` accepts it and the decoder reads the change.
fn read_bit_flips(d: &PpuGenerationDescriptor, index: usize) -> Vec<u32> {
    let canonical = canonical_parameters(d.canonical_word, &d.operands);
    let base = decode(d.canonical_word).expect("a canonical word decodes");
    let bits = d.operands[index].mask.count_ones();
    (0..bits)
        .filter_map(|bit| {
            let mut values = canonical.clone();
            values[index] ^= 1 << bit;
            d.encode(&values).ok()
        })
        .filter(|&word| decode(word).ok() != Some(base))
        .collect()
}

#[test]
fn every_operand_bit_the_decoder_reads_changes_the_text() {
    let mut hidden = Vec::new();
    let mut checked = 0;
    for d in &generation_descriptors() {
        // A refused word keeps its raw bits, reserved bits included,
        // and its text shows only the operands of its mnemonic.
        if matches!(
            d.kind,
            PpuFuzzKind::Ordinary(
                PpuInstructionKind::PpeIllegal | PpuInstructionKind::PpePrivileged
            )
        ) {
            continue;
        }
        let base = text(d.canonical_word);
        for (index, field) in d.operands.iter().enumerate() {
            let flips = read_bit_flips(d, index);
            if flips.is_empty() {
                continue;
            }
            checked += 1;
            let shown: Vec<bool> = flips.iter().map(|&word| text(word) != base).collect();
            let ok = if is_hinted_bo(d, field.mask, field.class) {
                shown.iter().any(|&s| s)
            } else {
                shown.iter().all(|&s| s)
            };
            if !ok {
                hidden.push(format!(
                    "{:?} field {:#010x} ({:?}): {base:?}",
                    d.kind, field.mask, field.class
                ));
            }
        }
    }
    assert!(checked > 600, "{checked} fields checked");
    assert!(
        hidden.is_empty(),
        "{} fields:\n{}",
        hidden.len(),
        hidden.join("\n")
    );
}
