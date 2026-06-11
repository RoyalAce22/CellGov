//! Spec-derived corpus for the AltiVec-memory load family (`lvsl`,
//! `lvsr`, `lvebx`, `lvehx`, `lvewx`, `lvxl`).
//!
//! For the element-indexed loads (`lvebx` / `lvehx` / `lvewx`) the
//! PEM lists unaffected lanes as architecturally "undefined";
//! CellGov's deterministic policy is to preserve prior VRT bytes
//! outside the written element, and the cases assert the full
//! 16-byte VRT to lock that policy.

use super::super::{InstructionCase, MemorySnapshot};
use super::{case_keep_memory, state_with_two_gprs};

/// Encode an X-form primary-31 instruction `(vt/vs, ra, rb, xo)`.
fn xform(vt_or_vs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    (31u32 << 26)
        | ((vt_or_vs as u32 & 0x1F) << 21)
        | ((ra as u32 & 0x1F) << 16)
        | ((rb as u32 & 0x1F) << 11)
        | (xo << 1)
}

const BASE_ADDR: u64 = 0x4000;

fn memory_with(bytes: Vec<u8>) -> MemorySnapshot {
    MemorySnapshot {
        base: BASE_ADDR,
        bytes,
    }
}

fn padded(offset: usize, payload: &[u8], total_len: usize) -> MemorySnapshot {
    let mut bytes = vec![0u8; total_len];
    bytes[offset..offset + payload.len()].copy_from_slice(payload);
    memory_with(bytes)
}

/// Spec-derived cases for the entire AltiVec-memory load family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(lvsl_cases());
    v.extend(lvsr_cases());
    v.extend(lvebx_cases());
    v.extend(lvehx_cases());
    v.extend(lvewx_cases());
    v.extend(lvxl_cases());
    v
}

/// `lvsl`: permute control vector. Bytes 0..=15 of VRT = sh, sh+1, ..., sh+15
/// where sh = EA[60:63]. No memory read.
// [AltiVec-PEM p:6-21 s:6.2] lvsl VRT, RA, RB: build shift-left permute control.
fn lvsl_cases() -> Vec<InstructionCase> {
    let raw = xform(/*vt*/ 1, /*ra*/ 4, /*rb*/ 5, 6);
    let mut cases = Vec::new();

    static LABELS: [&str; 16] = [
        "lvsl_sh_0",
        "lvsl_sh_1",
        "lvsl_sh_2",
        "lvsl_sh_3",
        "lvsl_sh_4",
        "lvsl_sh_5",
        "lvsl_sh_6",
        "lvsl_sh_7",
        "lvsl_sh_8",
        "lvsl_sh_9",
        "lvsl_sh_10",
        "lvsl_sh_11",
        "lvsl_sh_12",
        "lvsl_sh_13",
        "lvsl_sh_14",
        "lvsl_sh_15",
    ];
    for sh in 0u64..16 {
        let initial = state_with_two_gprs(4, BASE_ADDR, 5, sh);
        let mut expected = initial.clone();
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = sh as u8 + i as u8;
        }
        expected.vr[1] = u128::from_be_bytes(bytes);
        cases.push(case_keep_memory(
            LABELS[sh as usize],
            raw,
            initial,
            MemorySnapshot::empty(),
            expected,
            MemorySnapshot::empty(),
            "[AltiVec-PEM p:6-21 s:6.2] VRT[i] = sh + i for i in 0..16",
        ));
    }

    cases
}

/// `lvsr`: shift-right companion. VRT[i] = 16 + i - sh for i in 0..16.
// [AltiVec-PEM p:6-22 s:6.2] lvsr VRT, RA, RB: build shift-right permute control.
fn lvsr_cases() -> Vec<InstructionCase> {
    let raw = xform(2, 4, 5, 38);
    let mut cases = Vec::new();

    static LABELS: [&str; 16] = [
        "lvsr_sh_0",
        "lvsr_sh_1",
        "lvsr_sh_2",
        "lvsr_sh_3",
        "lvsr_sh_4",
        "lvsr_sh_5",
        "lvsr_sh_6",
        "lvsr_sh_7",
        "lvsr_sh_8",
        "lvsr_sh_9",
        "lvsr_sh_10",
        "lvsr_sh_11",
        "lvsr_sh_12",
        "lvsr_sh_13",
        "lvsr_sh_14",
        "lvsr_sh_15",
    ];
    for sh in 0u64..16 {
        let initial = state_with_two_gprs(4, BASE_ADDR, 5, sh);
        let mut expected = initial.clone();
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (16i32 + i as i32 - sh as i32) as u8;
        }
        expected.vr[2] = u128::from_be_bytes(bytes);
        cases.push(case_keep_memory(
            LABELS[sh as usize],
            raw,
            initial,
            MemorySnapshot::empty(),
            expected,
            MemorySnapshot::empty(),
            "[AltiVec-PEM p:6-22 s:6.2] VRT[i] = 16 + i - sh for i in 0..16",
        ));
    }

    cases
}

/// `lvebx`: byte load at EA into byte position (EA & 0xF) of VRT.
/// CellGov preserves the other 15 byte lanes (deterministic policy
/// for the architecturally-undefined bytes).
// [AltiVec-PEM p:6-15 s:6.2] lvebx VRT, RA, RB: 1-byte element load.
fn lvebx_cases() -> Vec<InstructionCase> {
    let raw = xform(3, 4, 5, 7);
    let mut cases = Vec::new();

    // Distinctive prior VRT so "lanes preserved" is verifiable.
    let initial_vrt: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;
    let prior_bytes = initial_vrt.to_be_bytes();

    for ea_offset in [0u64, 1, 7, 15] {
        let payload = 0xA5u8;
        let mem = padded(ea_offset as usize, &[payload], 16);
        let mut initial = state_with_two_gprs(4, BASE_ADDR, 5, ea_offset);
        initial.vr[3] = initial_vrt;
        let mut expected = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut bytes = prior_bytes;
        bytes[m] = payload;
        expected.vr[3] = u128::from_be_bytes(bytes);
        let label: &'static str = match ea_offset {
            0 => "lvebx_offset_0",
            1 => "lvebx_offset_1",
            7 => "lvebx_offset_7",
            15 => "lvebx_offset_15",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            mem.clone(),
            expected,
            mem,
            "[AltiVec-PEM p:6-15 s:6.2] byte at EA -> VRT[8m..8m+7]; other lanes preserved per CellGov policy",
        ));
    }

    cases
}

/// `lvehx`: halfword load at (EA & ~1) into halfword position
/// `(EA & 0xE)` of VRT.
// [AltiVec-PEM p:6-16 s:6.2] lvehx VRT, RA, RB: 2-byte element load, EA aligned down to halfword.
fn lvehx_cases() -> Vec<InstructionCase> {
    let raw = xform(4, 4, 5, 39);
    let mut cases = Vec::new();

    let initial_vrt: u128 = 0xAAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000_1111;
    let prior_bytes = initial_vrt.to_be_bytes();
    let payload = [0xC0u8, 0xDE];

    for ea_offset in [0u64, 2, 8, 14] {
        let mem = padded(ea_offset as usize, &payload, 16);
        let mut initial = state_with_two_gprs(4, BASE_ADDR, 5, ea_offset);
        initial.vr[4] = initial_vrt;
        let mut expected = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut bytes = prior_bytes;
        bytes[m] = payload[0];
        bytes[m + 1] = payload[1];
        expected.vr[4] = u128::from_be_bytes(bytes);
        let label: &'static str = match ea_offset {
            0 => "lvehx_offset_0",
            2 => "lvehx_offset_2",
            8 => "lvehx_offset_8",
            14 => "lvehx_offset_14",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            mem.clone(),
            expected,
            mem,
            "[AltiVec-PEM p:6-16 s:6.2] halfword at (EA&~1) -> VRT lane (EA&0xE); other lanes preserved",
        ));
    }

    cases
}

/// `lvewx`: word load at (EA & ~3) into word position `(EA & 0xC)` of VRT.
// [AltiVec-PEM p:6-17 s:6.2] lvewx VRT, RA, RB: 4-byte element load, EA aligned down to word.
fn lvewx_cases() -> Vec<InstructionCase> {
    let raw = xform(5, 4, 5, 71);
    let mut cases = Vec::new();

    let initial_vrt: u128 = 0x1111_2222_3333_4444_5555_6666_7777_8888;
    let prior_bytes = initial_vrt.to_be_bytes();
    let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];

    for ea_offset in [0u64, 4, 8, 12] {
        let mem = padded(ea_offset as usize, &payload, 16);
        let mut initial = state_with_two_gprs(4, BASE_ADDR, 5, ea_offset);
        initial.vr[5] = initial_vrt;
        let mut expected = initial.clone();
        let m = (ea_offset & 0xF) as usize;
        let mut bytes = prior_bytes;
        bytes[m..m + 4].copy_from_slice(&payload);
        expected.vr[5] = u128::from_be_bytes(bytes);
        let label: &'static str = match ea_offset {
            0 => "lvewx_offset_0",
            4 => "lvewx_offset_4",
            8 => "lvewx_offset_8",
            12 => "lvewx_offset_12",
            _ => unreachable!(),
        };
        cases.push(case_keep_memory(
            label,
            raw,
            initial,
            mem.clone(),
            expected,
            mem,
            "[AltiVec-PEM p:6-17 s:6.2] word at (EA&~3) -> VRT lane (EA&0xC); other lanes preserved",
        ));
    }

    cases
}

/// `lvxl`: identical to `lvx` -- the "Last" suffix is a cache LRU hint.
// [AltiVec-PEM p:6-23 s:6.2] lvxl VRT, RA, RB: same semantics as lvx with LRU hint.
fn lvxl_cases() -> Vec<InstructionCase> {
    let raw = xform(6, 4, 5, 359);
    let payload = [
        0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];
    let mem = padded(0, &payload, 32);
    let initial = state_with_two_gprs(4, BASE_ADDR, 5, 0);
    let mut expected = initial.clone();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&payload);
    expected.vr[6] = u128::from_be_bytes(bytes);
    vec![case_keep_memory(
        "lvxl_aligned_at_offset_0",
        raw,
        initial,
        mem.clone(),
        expected,
        mem,
        "[AltiVec-PEM p:6-23 s:6.2] lvxl: 16-byte aligned load; LRU hint ignored",
    )]
}

#[cfg(test)]
mod tests {
    use super::super::super::{assert_case, run_corpus};
    use super::*;

    #[test]
    fn altivec_memory_load_corpus_passes_against_executor() {
        let cases = cases();
        assert!(
            !cases.is_empty(),
            "AltiVec-memory load corpus must produce at least one case"
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
                "AltiVec-memory load corpus: {} failure(s) of {}:\n{detail}",
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

    #[test]
    fn corpus_covers_all_six_ops() {
        let cases = cases();
        let labels: Vec<&str> = cases.iter().map(|c| c.label).collect();
        for prefix in ["lvsl_", "lvsr_", "lvebx_", "lvehx_", "lvewx_", "lvxl_"] {
            assert!(
                labels.iter().any(|l| l.starts_with(prefix)),
                "corpus missing any '{prefix}' case"
            );
        }
    }
}
