//! The `spu_float_edges` microtest's cases run on CellGov's SPU in the gate.
//!
//! `tests/micro/spu_float_edges/cases.tsv` is the one source: each case's
//! instructions, inputs and expected `$3` and FPSCR. This test assembles
//! each case, runs `fscrwr`, the case and `fscrrd` on a fresh SPU state,
//! and compares. The ignored `regenerate` test fills the expected columns
//! and writes the SPU program the hardware and RPCS3 runs build from.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu_isa::{
    row_named, SpuForm, SPU_OPCODE_MAP, TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS,
};
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::SpuState;

const CASES: &str = "../../tests/micro/spu_float_edges/cases.tsv";

/// The register the harness writes the FPSCR from and reads it into.
const FPSCR_IN: u32 = 20;
const FPSCR_OUT: u32 = 21;

struct Case {
    name: String,
    asm: Vec<String>,
    fpscr: u128,
    inputs: [u128; 4],
    expect: Option<(u128, u128)>,
    source: String,
}

fn parse_value(field: &str) -> u128 {
    u128::from_str_radix(&field.replace('_', ""), 16).unwrap_or_else(|e| panic!("{field}: {e}"))
}

fn format_value(value: u128) -> String {
    let hex = format!("{value:032x}");
    [0, 8, 16, 24].map(|i| &hex[i..i + 8]).join("_")
}

fn read_cases(text: &str) -> Vec<Case> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with("case\t") && !line.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            assert_eq!(f.len(), 10, "{line}");
            let expect = (f[7] != "?").then(|| (parse_value(f[7]), parse_value(f[8])));
            Case {
                name: f[0].to_string(),
                asm: f[1].split(';').map(|s| s.trim().to_string()).collect(),
                fpscr: parse_value(f[2]),
                inputs: [f[3], f[4], f[5], f[6]].map(parse_value),
                expect,
                source: f[9].to_string(),
            }
        })
        .collect()
}

fn register(operand: &str) -> u32 {
    operand
        .strip_prefix('$')
        .and_then(|n| n.parse().ok())
        .filter(|n: &u32| *n < 128)
        .unwrap_or_else(|| panic!("{operand} is not a register"))
}

fn immediate(operand: &str) -> u32 {
    match operand.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => operand.parse(),
    }
    .unwrap_or_else(|e| panic!("{operand}: {e}"))
}

/// Encodes one instruction in SPU assembler syntax.
// [SPU-ISA p:28 s:2.3] RR, RRR; [SPU-ISA p:29 s:2.3] RI16; [SPU-ISA p:220 s:9] RI8, whose assembler operand is the scale.
fn assemble(line: &str) -> u32 {
    let (mnemonic, operands) = line.split_once(' ').unwrap_or((line, ""));
    let ops: Vec<&str> = operands
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let row = &SPU_OPCODE_MAP[row_named(mnemonic).unwrap_or_else(|| panic!("{mnemonic}"))];
    let base = row.canonical_word();
    match (row.form, mnemonic, ops.as_slice()) {
        (SpuForm::Rr, "fscrwr", [ra]) => base | register(ra) << 7,
        (SpuForm::Rr, _, [rt]) => base | register(rt),
        (SpuForm::Rr, _, [rt, ra]) => base | register(ra) << 7 | register(rt),
        (SpuForm::Rr, _, [rt, ra, rb]) => {
            base | register(rb) << 14 | register(ra) << 7 | register(rt)
        }
        (SpuForm::Rrr, _, [rt, ra, rb, rc]) => {
            base | register(rt) << 21 | register(rb) << 14 | register(ra) << 7 | register(rc)
        }
        (SpuForm::Ri8, _, [rt, ra, scale]) => {
            let bias = if mnemonic.starts_with("cf") {
                TO_INTEGER_SCALE_BIAS
            } else {
                TO_FLOAT_SCALE_BIAS
            };
            let scale = immediate(scale);
            // [SPU-ISA p:220 s:9] a scale outside 0 to 127 gives an undefined result.
            assert!(scale <= 127, "{line}: scale {scale} is outside 0..=127");
            let i8 = u32::from(bias) - scale;
            base | i8 << 14 | register(ra) << 7 | register(rt)
        }
        (SpuForm::Ri16, _, [rt, imm]) => {
            let imm = immediate(imm);
            assert!(imm <= 0xFFFF, "{line}: {imm:#x} does not fit I16");
            base | imm << 7 | register(rt)
        }
        _ => panic!("cannot assemble {line}"),
    }
}

fn step(state: &mut SpuState, line: &str) {
    let insn =
        cellgov_spu::decode::decode(assemble(line)).unwrap_or_else(|e| panic!("{line}: {e}"));
    assert_eq!(
        execute(&insn, state, UnitId::new(0)),
        SpuStepOutcome::Continue,
        "{line}"
    );
}

/// Runs one case the way the microtest does: the FPSCR written, the case,
/// the FPSCR read; returns `$3` and the FPSCR read back.
fn run(case: &Case) -> (u128, u128) {
    let mut state = SpuState::new();
    for (register, value) in (3..).zip(case.inputs) {
        state.regs[register] = value.to_be_bytes();
    }
    state.regs[FPSCR_IN as usize] = case.fpscr.to_be_bytes();
    step(&mut state, &format!("fscrwr ${FPSCR_IN}"));
    for line in &case.asm {
        step(&mut state, line);
    }
    step(&mut state, &format!("fscrrd ${FPSCR_OUT}"));
    (
        u128::from_be_bytes(state.regs[3]),
        u128::from_be_bytes(state.regs[FPSCR_OUT as usize]),
    )
}

#[test]
fn every_case_matches_its_expected_result() {
    let cases = read_cases(&std::fs::read_to_string(CASES).expect("cases.tsv"));
    assert!(cases.len() >= 20, "{} cases", cases.len());
    for case in &cases {
        let (want_r3, want_fpscr) = case.expect.unwrap_or_else(|| {
            panic!(
                "{} has no expected result; run the regenerate test",
                case.name
            )
        });
        let (r3, fpscr) = run(case);
        assert_eq!(
            (format_value(r3), format_value(fpscr)),
            (format_value(want_r3), format_value(want_fpscr)),
            "{}: {}",
            case.name,
            case.source
        );
    }
}

/// The SPU program: per case, the inputs loaded, the FPSCR written, the
/// case run, and `$3` and the FPSCR stored to `results`.
fn program(cases: &[Case]) -> String {
    let mut data = String::new();
    let mut text = String::new();
    for (index, case) in cases.iter().enumerate() {
        let input = index * 80;
        let output = index * 32;
        data.push_str(&format!("\t# {}\n", case.name));
        for value in case.inputs.iter().chain([&case.fpscr]) {
            let words: Vec<String> = (0..4)
                .map(|i| format!("0x{:08x}", (value >> (96 - 32 * i)) as u32))
                .collect();
            data.push_str(&format!("\t.long {}\n", words.join(",")));
        }
        text.push_str(&format!("\t# {}\n", case.name));
        for register in 3..7 {
            text.push_str(&format!(
                "\tlqa ${register},inputs+{}\n",
                input + (register - 3) * 16
            ));
        }
        text.push_str(&format!("\tlqa ${FPSCR_IN},inputs+{}\n", input + 64));
        text.push_str(&format!("\tfscrwr ${FPSCR_IN}\n"));
        for line in &case.asm {
            text.push_str(&format!("\t{line}\n"));
        }
        text.push_str(&format!("\tfscrrd ${FPSCR_OUT}\n"));
        text.push_str(&format!("\tstqa $3,results+{output}\n"));
        text.push_str(&format!("\tstqa ${FPSCR_OUT},results+{}\n", output + 16));
    }
    format!(
        "# Generated from ../cases.tsv by the regenerate test in\n\
         # crates/cellgov_spu/tests/spu_float_edges.rs. Do not edit.\n\
         \t.data\n\t.align 4\ninputs:\n{data}\
         \t.align 4\n\t.global results\nresults:\n\t.space {}\n\
         \t.text\n\t.align 3\n\t.global run_cases\nrun_cases:\n{text}\tbi $0\n",
        cases.len() * 32
    )
}

#[test]
#[ignore = "rewrites cases.tsv's expected columns and the generated SPU program"]
fn regenerate() {
    let text = std::fs::read_to_string(CASES).expect("cases.tsv");
    let cases = read_cases(&text);
    let mut out = String::new();
    let mut rows = cases.iter();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("case\t") || line.is_empty() {
            out.push_str(line);
        } else {
            let case = rows.next().expect("a case per row");
            let (r3, fpscr) = run(case);
            let mut fields: Vec<String> = line.split('\t').map(str::to_string).collect();
            fields[7] = format_value(r3);
            fields[8] = format_value(fpscr);
            out.push_str(&fields.join("\t"));
        }
        out.push('\n');
    }
    std::fs::write(CASES, out).expect("write cases.tsv");
    std::fs::write(
        "../../tests/micro/spu_float_edges/spu/cases.S",
        program(&cases),
    )
    .expect("write cases.S");
    std::fs::write(
        "../../tests/micro/spu_float_edges/cases.h",
        format!(
            "/* Generated from cases.tsv. Do not edit. */\n#define CASE_COUNT {}\n#define RESULT_BYTES {}\n",
            cases.len(),
            cases.len() * 32
        ),
    )
    .expect("write cases.h");
}

/// The built program, run on CellGov's SPU from `main`, DMAs out exactly
/// the table's expected results.
#[test]
#[cfg_attr(
    not(feature = "spu-microtests"),
    ignore = "needs the built microtest (tests/micro/spu_float_edges/build.sh); run with --features spu-microtests"
)]
fn the_built_program_stores_every_expected_result() {
    use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
    use cellgov_time::Budget;

    let cases = read_cases(&std::fs::read_to_string(CASES).expect("cases.tsv"));
    let path = "../../tests/micro/spu_float_edges/build/spu_main.elf";
    let elf = std::fs::read(path).unwrap_or_else(|e| {
        panic!("{path}: {e}; build it with tests/micro/spu_float_edges/build.sh")
    });
    let mut unit = cellgov_spu::SpuExecutionUnit::new(UnitId::new(1));
    cellgov_spu::loader::load_spu_elf(&elf, unit.state_mut()).expect("loads");
    // main(speid, argp, envp): the stack in $1 and the result EA in $4.
    let result_ea: u32 = 0x1_0000;
    unit.state_mut().pc = 0x80;
    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
    unit.state_mut().set_reg_word_splat(4, result_ea);
    let mem = cellgov_mem::GuestMemory::new(0x2_0000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    for _ in 0..1_000 {
        let mut step = Vec::new();
        let result = unit.run_until_yield(Budget::new(100_000), &ctx, &mut step);
        effects.extend(step);
        match result.yield_reason {
            YieldReason::Finished => break,
            YieldReason::DmaSubmitted | YieldReason::BudgetExhausted | YieldReason::DmaWait => {}
            other => panic!("unexpected yield {other:?}: {:?}", result.fault),
        }
    }
    let payload = effects
        .iter()
        .find_map(|effect| match effect {
            cellgov_effects::Effect::DmaEnqueue {
                request, payload, ..
            } if request.destination().start().raw() == u64::from(result_ea) => payload.clone(),
            _ => None,
        })
        .expect("the results DMA put");
    for (index, case) in cases.iter().enumerate() {
        let (want_r3, want_fpscr) = case.expect.expect("expected results");
        let at = |offset: usize| {
            u128::from_be_bytes(payload[offset..offset + 16].try_into().expect("16 bytes"))
        };
        assert_eq!(
            (
                format_value(at(index * 32)),
                format_value(at(index * 32 + 16))
            ),
            (format_value(want_r3), format_value(want_fpscr)),
            "{}",
            case.name
        );
    }
}

const PEER: &str = "../../tests/micro/spu_float_edges/peer.tsv";

/// RPCS3's results for one decoder: per case, `$3` and the FPSCR.
fn rpcs3_results(decoder: &str) -> Vec<(u128, u128)> {
    let path = format!("../../tests/scenario_observations/spu_float_edges/rpcs3_{decoder}.json");
    let observation = cellgov_compare::baseline::load(std::path::Path::new(&path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    observation.memory_regions[0]
        .data
        .chunks(32)
        .map(|case| {
            let quad =
                |at: usize| u128::from_be_bytes(case[at..at + 16].try_into().expect("16 bytes"));
            (quad(0), quad(16))
        })
        .collect()
}

/// RPCS3 is a differential peer, not the reference: every case where it
/// disagrees with the expected result has a row in peer.tsv with its
/// class, and every row still disagrees.
// [McKeeman1998 p:101 s:Differential Testing] two implementations can differ and both stay within the documents, so a disagreement is classified, not failed on.
#[test]
fn every_rpcs3_disagreement_is_classified() {
    let cases = read_cases(&std::fs::read_to_string(CASES).expect("cases.tsv"));
    let mut found = std::collections::BTreeSet::new();
    for decoder in ["interpreter", "llvm"] {
        let peer = rpcs3_results(decoder);
        assert_eq!(peer.len(), cases.len(), "{decoder} holds every case");
        for (case, (r3, fpscr)) in cases.iter().zip(peer) {
            let (want_r3, want_fpscr) = case.expect.expect("expected results");
            for (field, got, want) in [("r3", r3, want_r3), ("fpscr", fpscr, want_fpscr)] {
                if got != want {
                    found.insert(format!(
                        "{}\t{decoder}\t{field}\t{}",
                        case.name,
                        format_value(got)
                    ));
                }
            }
        }
    }
    let text = std::fs::read_to_string(PEER).expect("peer.tsv");
    let mut listed = std::collections::BTreeSet::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("case\t") && !l.is_empty())
    {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "{line}");
        assert!(
            [
                "peer-inaccuracy",
                "cellgov-defect",
                "implementation-defined"
            ]
            .contains(&fields[4]),
            "{line}"
        );
        listed.insert(fields[..4].join("\t"));
    }
    let unlisted: Vec<&String> = found.difference(&listed).collect();
    let stale: Vec<&String> = listed.difference(&found).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "unclassified RPCS3 disagreements:\n{}\nlisted but no longer disagreeing:\n{}",
        unlisted
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        stale
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
