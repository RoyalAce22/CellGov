//! The harness's own checks: every committed header is its table
//! rendered, every row's text is its words, and the table refuses what
//! the probe routine cannot run.

use cellgov_ppu::decode::decode;
use cellgov_ppu::instruction::AsmText;

use super::{parse_table, probe_dir, render, table, PROBES};

/// The probe's assembly text for each word, as CellGov renders it.
fn rendered(words: &[u32]) -> String {
    words
        .iter()
        .map(|&w| match decode(w) {
            Ok(insn) => AsmText {
                insn: &insn,
                addr: 0,
                symbols: None,
            }
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
            Err(e) => format!("<{e}>"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn each_probe_header_is_its_table_rendered() {
    let regenerate = std::env::var_os("CELLGOV_REGENERATE").is_some();
    for probe in PROBES {
        let want = render(probe, &table(probe));
        let path = probe_dir(probe).join("ppu/cases.h");
        if regenerate {
            std::fs::write(&path, &want).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        let have =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            have == want,
            "{}: stale; rerun with CELLGOV_REGENERATE=1",
            path.display()
        );
    }
}

#[test]
fn each_row_asm_text_is_its_words() {
    let mut rows = 0;
    let mut wrong = Vec::new();
    for probe in PROBES {
        for case in table(probe) {
            rows += 1;
            if case.may_end_process {
                continue;
            }
            let want = case
                .asm
                .split(';')
                .map(|w| w.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("; ");
            let have = rendered(&case.words);
            if have != want {
                wrong.push(format!(
                    "{probe} {}: table {want:?}, words {have:?}",
                    case.id
                ));
            }
        }
    }
    assert!(rows > 0);
    assert!(
        wrong.is_empty(),
        "{} row(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

fn refusal(inputs: &str, record: &str) -> String {
    let text = format!(
        "id\tasm\twords\tinputs\trecord\tmay_end_process\na\tnop\t0x60000000\t{inputs}\t{record}\t0\n"
    );
    parse_table(&text).expect_err(&format!("{inputs} / {record}"))
}

#[test]
fn a_table_refuses_a_fatal_case_beside_another() {
    let text = "id\tasm\twords\tinputs\trecord\tmay_end_process\n\
                a\tnop\t0x60000000\t-\t-\t1\n\
                b\tnop\t0x60000000\t-\t-\t0\n";
    let err = parse_table(text).expect_err("two cases, one fatal");
    assert!(err.contains("only case"), "{err}");
}

#[test]
fn a_table_refuses_the_routine_registers() {
    for reg in [
        "r11", "r12", "r0", "r13", "f0", "f14", "v0", "v14", "m4", "m256",
    ] {
        let err = refusal("-", reg);
        assert!(err.contains("not one a case may use"), "{reg}: {err}");
    }
}

#[test]
fn a_table_refuses_memory_it_cannot_place() {
    assert!(refusal("-", "m0").contains("needs mem="));
    assert!(refusal("mem=0x00", "r10").contains("holds the memory buffer"));
    assert!(refusal("mem=0x00,r10=0x1", "-").contains("holds the memory buffer"));
    assert!(refusal("mem=0x00,mem=0x11", "-").contains("loaded twice"));
    let too_long = format!("mem=0x{}", "00".repeat(257));
    assert!(refusal(&too_long, "-").contains("hex digits"));
}

#[test]
fn a_table_refuses_an_empty_or_non_ascii_register_name() {
    assert!(refusal("=0x1", "-").contains("not one a case may use"));
    assert!(refusal("-", "\u{e9}3").contains("not one a case may use"));
}

#[test]
fn a_table_refuses_non_ascii_memory_digits() {
    assert!(refusal("mem=0xa\u{e9}0", "-").contains("not hex"));
}

#[test]
fn a_table_refuses_records_past_the_slots() {
    // Two VRs fill the four slots, and a VR skips an odd slot.
    assert!(refusal("-", "v1,v2,r3").contains("slots"));
    assert!(refusal("-", "r3,v1,r4").contains("slots"));
}
