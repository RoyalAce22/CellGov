//! Spec-derived corpus for the byte-reverse indexed family
//! (`ldbrx`, `lwbrx`, `lhbrx`, `sdbrx`, `stwbrx`, `sthbrx`).
//!
//! The cases verify CellGov's executor against the PowerPC
//! definition of each op: a `(RA|0) + RB` effective address, a
//! little-endian load of N bytes (zero-extended to RT for the load
//! variants), and a little-endian store of the low N bytes of RS
//! for the store variants. Citations on each case route through the
//! [`super::OracleSource::Spec`] `rationale` field.

use super::super::{InstructionCase, MemorySnapshot, OracleSource};
use super::{case_keep_memory, state_with_gpr, state_with_three_gprs, state_with_two_gprs};

/// Encode an X-form primary-31 instruction `(rt/rs, ra, rb, xo)`.
fn xform(rt_or_rs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((rt_or_rs as u32 & 0x1F) << 21)
        | ((ra as u32 & 0x1F) << 16)
        | ((rb as u32 & 0x1F) << 11)
        | (xo << 1)
}

/// Spec-derived cases for the entire byte-reverse family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(ldbrx_cases());
    v.extend(lwbrx_cases());
    v.extend(lhbrx_cases());
    v.extend(sdbrx_cases());
    v.extend(stwbrx_cases());
    v.extend(sthbrx_cases());
    v
}

const BASE_ADDR: u64 = 0x4000;

fn make_memory(bytes: Vec<u8>) -> MemorySnapshot {
    MemorySnapshot {
        base: BASE_ADDR,
        bytes,
    }
}

fn padded_memory(offset: usize, payload: &[u8], total_len: usize) -> MemorySnapshot {
    let mut bytes = vec![0u8; total_len];
    bytes[offset..offset + payload.len()].copy_from_slice(payload);
    make_memory(bytes)
}

/// `ldbrx`: 8-byte big-endian-on-disk load that the executor must
/// byte-reverse into RT.
// [PPC-Book1 p:51 s:3.3.4] ldbrx RT, RA, RB: MEM(EA,8) byte-reversed -> RT.
fn ldbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(/*rt*/ 3, /*ra*/ 4, /*rb*/ 5, 532);
    let mut cases = Vec::new();

    // Case 1: pattern 00..07 at offset 0; expected RT is the
    // little-endian read = 0x0706050403020100.
    let mem = padded_memory(0, &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07], 32);
    let initial = state_with_two_gprs(4, BASE_ADDR, 5, 0);
    let mut expected = initial.clone();
    expected.gpr[3] = 0x0706_0504_0302_0100;
    cases.push(case_keep_memory(
        "ldbrx_ascending_at_offset_0",
        raw,
        initial,
        mem.clone(),
        expected,
        mem,
        "[PPC-Book1 p:51 s:3.3.4] little-endian 8-byte load -> RT",
    ));

    // Case 2: same pattern at EA = base + 8 via RB.
    let mut mem_bytes = vec![0u8; 32];
    mem_bytes[8..16].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE]);
    let mem2 = make_memory(mem_bytes);
    let initial2 = state_with_two_gprs(4, BASE_ADDR, 5, 8);
    let mut expected2 = initial2.clone();
    expected2.gpr[3] = u64::from_le_bytes([0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE]);
    cases.push(case_keep_memory(
        "ldbrx_deadbeefcafebabe_at_offset_8",
        raw,
        initial2,
        mem2.clone(),
        expected2,
        mem2,
        "[PPC-Book1 p:51 s:3.3.4] EA = (RA|0)+RB; little-endian 8-byte load",
    ));

    // Case 3: RA=0, EA = RB. We re-encode with RA=0.
    let raw_ra0 = xform(3, 0, 5, 532);
    let mut mem_bytes = vec![0u8; 32];
    mem_bytes[16..24].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let mem3 = make_memory(mem_bytes);
    let initial3 = state_with_gpr(5, BASE_ADDR + 16);
    let mut expected3 = initial3.clone();
    expected3.gpr[3] = u64::from_le_bytes([0xFF; 8]);
    cases.push(case_keep_memory(
        "ldbrx_ra0_ea_is_rb_only",
        raw_ra0,
        initial3,
        mem3.clone(),
        expected3,
        mem3,
        "[PPC-Book1 p:33 s:3.3.2] RA=0 means EA omits the RA term",
    ));

    cases
}

/// `lwbrx`: 4-byte little-endian load zero-extended into RT.
// [PPC-Book1 p:50 s:3.3.4] lwbrx RT, RA, RB: low 32 bits byte-reversed -> RT[32:63]; RT[0:31]=0.
fn lwbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(7, 8, 9, 534);
    let mut cases = Vec::new();

    // Aligned 4-byte at offset 4, pattern 0x11_22_33_44.
    let mut mem_bytes = vec![0u8; 32];
    mem_bytes[4..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    let mem = make_memory(mem_bytes);
    let initial = state_with_two_gprs(8, BASE_ADDR, 9, 4);
    let mut expected = initial.clone();
    expected.gpr[7] = 0x4433_2211u32 as u64;
    cases.push(case_keep_memory(
        "lwbrx_pattern_at_offset_4",
        raw,
        initial,
        mem.clone(),
        expected,
        mem,
        "[PPC-Book1 p:50 s:3.3.4] little-endian 4-byte load, zero-extended into RT",
    ));

    // Mid-region misalignment: EA at offset 5.
    let mut mem_bytes = vec![0u8; 32];
    mem_bytes[5..9].copy_from_slice(&[0xAB, 0xCD, 0xEF, 0x01]);
    let mem2 = make_memory(mem_bytes);
    let initial2 = state_with_two_gprs(8, BASE_ADDR, 9, 5);
    let mut expected2 = initial2.clone();
    expected2.gpr[7] = 0x01EF_CDABu32 as u64;
    cases.push(case_keep_memory(
        "lwbrx_misaligned_at_offset_5",
        raw,
        initial2,
        mem2.clone(),
        expected2,
        mem2,
        "[PPC-Book1 p:50 s:3.3.4] EA need not be aligned for the byte-reverse loads",
    ));

    cases
}

/// `lhbrx`: 2-byte little-endian load zero-extended.
// [PPC-Book1 p:50 s:3.3.4] lhbrx RT, RA, RB: low 16 bits byte-reversed -> RT[48:63]; RT[0:47]=0.
fn lhbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(2, 4, 5, 790);
    let mut cases = Vec::new();

    let mut mem_bytes = vec![0u8; 16];
    mem_bytes[6..8].copy_from_slice(&[0xAB, 0xCD]);
    let mem = make_memory(mem_bytes);
    let initial = state_with_two_gprs(4, BASE_ADDR, 5, 6);
    let mut expected = initial.clone();
    expected.gpr[2] = 0xCDAB;
    cases.push(case_keep_memory(
        "lhbrx_pattern_at_offset_6",
        raw,
        initial,
        mem.clone(),
        expected,
        mem,
        "[PPC-Book1 p:50 s:3.3.4] little-endian 2-byte load, zero-extended into RT",
    ));

    cases
}

/// `sdbrx`: 8-byte little-endian store from RS.
// [CBE-Handbook p:734 s:A.2.1] sdbrx RS, RA, RB: RS byte-reversed -> MEM(EA,8).
fn sdbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(/*rs*/ 6, 7, 8, 660);
    let mut cases = Vec::new();

    let initial = state_with_three_gprs((6, 0x0123_4567_89AB_CDEFu64), (7, BASE_ADDR), (8, 0));
    let initial_mem = padded_memory(0, &[0u8; 8], 32);
    let mut expected_mem_bytes = vec![0u8; 32];
    expected_mem_bytes[0..8].copy_from_slice(&[0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01]);
    let expected_mem = make_memory(expected_mem_bytes);
    cases.push(InstructionCase {
        label: "sdbrx_pattern_at_offset_0",
        initial_state: initial.clone(),
        initial_memory: initial_mem,
        raw_instruction: raw,
        expected_state: initial,
        expected_memory: expected_mem,
        source: OracleSource::Spec {
            rationale: "[CBE-Handbook p:734 s:A.2.1] sdbrx 8-byte little-endian store",
        },
    });

    cases
}

/// `stwbrx`: 4-byte little-endian store from low 32 of RS.
// [PPC-Book1 p:51 s:3.3.4] stwbrx: RS[32:63] byte-reversed -> MEM(EA,4).
fn stwbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(9, 10, 11, 662);
    let mut cases = Vec::new();

    let initial = state_with_three_gprs((9, 0xFFFF_FFFF_1122_3344u64), (10, BASE_ADDR), (11, 4));
    let initial_mem = padded_memory(0, &[0u8; 16], 16);
    let mut expected_mem_bytes = vec![0u8; 16];
    expected_mem_bytes[4..8].copy_from_slice(&[0x44, 0x33, 0x22, 0x11]);
    let expected_mem = make_memory(expected_mem_bytes);
    cases.push(InstructionCase {
        label: "stwbrx_low32_at_offset_4",
        initial_state: initial.clone(),
        initial_memory: initial_mem,
        raw_instruction: raw,
        expected_state: initial,
        expected_memory: expected_mem,
        source: OracleSource::Spec {
            rationale: "[PPC-Book1 p:51 s:3.3.4] stwbrx low-32 little-endian store",
        },
    });

    cases
}

/// `sthbrx`: 2-byte little-endian store from low 16 of RS.
// [PPC-Book1 p:51 s:3.3.4] sthbrx: RS[48:63] byte-reversed -> MEM(EA,2).
fn sthbrx_cases() -> Vec<InstructionCase> {
    let raw = xform(12, 13, 14, 918);
    let mut cases = Vec::new();

    let initial = state_with_three_gprs((12, 0xDEAD_BEEF_CAFE_BABE), (13, BASE_ADDR), (14, 2));
    let initial_mem = padded_memory(0, &[0u8; 8], 8);
    let mut expected_mem_bytes = vec![0u8; 8];
    expected_mem_bytes[2..4].copy_from_slice(&[0xBE, 0xBA]);
    let expected_mem = make_memory(expected_mem_bytes);
    cases.push(InstructionCase {
        label: "sthbrx_low16_at_offset_2",
        initial_state: initial.clone(),
        initial_memory: initial_mem,
        raw_instruction: raw,
        expected_state: initial,
        expected_memory: expected_mem,
        source: OracleSource::Spec {
            rationale: "[PPC-Book1 p:51 s:3.3.4] sthbrx low-16 little-endian store",
        },
    });

    cases
}

#[cfg(test)]
mod tests {
    use super::super::super::{assert_case, run_corpus};
    use super::*;

    #[test]
    fn byte_reverse_corpus_passes_against_executor() {
        let cases = cases();
        assert!(
            !cases.is_empty(),
            "byte-reverse corpus must produce at least one case"
        );
        let report = run_corpus(&cases);
        if !report.is_clean() {
            let detail = report
                .failed
                .iter()
                .map(|(label, outcome)| format!("  '{label}': {outcome:?}"))
                .collect::<Vec<_>>()
                .join("\n");
            panic!(
                "byte-reverse corpus: {} failure(s) of {}:\n{detail}",
                report.failed.len(),
                report.total()
            );
        }
    }

    #[test]
    fn each_case_passes_through_assert_case() {
        for case in cases() {
            assert_case(&case);
        }
    }
}
