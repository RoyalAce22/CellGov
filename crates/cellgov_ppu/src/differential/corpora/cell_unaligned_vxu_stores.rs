//! Spec-derived corpus for the Cell-unaligned VXU store family
//! (`stvlx`, `stvrx`, `stvlxl`, `stvrxl`).
//!
//! `stvlx` writes the high `16 - (EA & 0xF)` bytes of VS starting at
//! EA; `stvrx` writes the low `EA & 0xF` bytes of VS at the aligned
//! line below EA. Used together at EA and EA+16, they store a full
//! unaligned 16-byte vector without crossing the 16-byte aligned
//! line boundary mid-instruction (the architectural property that
//! gives the Cell PPE unaligned vector access without the AltiVec
//! `lvsl`/`vperm` shuffle).
//!
//! The "Last" suffixed twins (`stvlxl` / `stvrxl`) carry an LRU
//! cache hint that CellGov's no-cache model ignores; the corpus
//! treats them as direct aliases of `stvlx` / `stvrx`.

use super::super::{InstructionCase, MemorySnapshot};
use super::case_keep_memory;
use crate::differential::PpuStateSnapshot;

// Extended opcodes (primary-31 X-form). Stride of 64 encodes the
// left/right + LRU-hint bits.
const XO_STVLX: u32 = 775;
const XO_STVRX: u32 = 839;
const XO_STVLXL: u32 = 903;
const XO_STVRXL: u32 = 967;

// Register assignments used throughout the corpus.
const RA_IDX: u8 = 4;
const RB_IDX: u8 = 5;
const VS_IDX_STVLX: u8 = 11;
const VS_IDX_STVRX: u8 = 12;
const VS_IDX_STVLXL: u8 = 13;
const VS_IDX_STVRXL: u8 = 14;

/// Encode an X-form primary-31 instruction `(vs, ra, rb, xo)`.
fn xform(vs: u8, ra: u8, rb: u8, xo: u32) -> u32 {
    debug_assert!(
        xo < (1 << 10),
        "xo {xo} exceeds 10-bit X-form extended opcode field"
    );
    (31u32 << 26)
        | ((vs as u32 & 0x1F) << 21)
        | ((ra as u32 & 0x1F) << 16)
        | ((rb as u32 & 0x1F) << 11)
        | ((xo & 0x3FF) << 1)
}

const BASE_ADDR: u64 = 0x4000;

fn memory_with(bytes: Vec<u8>) -> MemorySnapshot {
    MemorySnapshot {
        base: BASE_ADDR,
        bytes,
    }
}

fn zero_memory(len: usize) -> MemorySnapshot {
    memory_with(vec![0u8; len])
}

/// Decompose an EA into (misalignment m, stvlx count, aligned base
/// offset): `m = EA & 0xF`, stvlx writes `16 - m` bytes at EA, stvrx
/// writes `m` bytes at `EA - m`.
fn ea_decompose(ea_offset: u64) -> (usize, usize, usize) {
    let m = (ea_offset & 0xF) as usize;
    (m, 16 - m, ea_offset as usize - m)
}

/// Label stem `{prefix}_offset_{ea}`, suffixed for the special byte
/// counts (full vector, single byte, empty store).
fn case_label(prefix: &str, ea_offset: u64, count: usize) -> String {
    let base = format!("{prefix}_offset_{ea_offset}");
    match count {
        16 => format!("{base}_aligned_full"),
        1 => format!("{base}_one_byte"),
        0 => format!("{base}_zero_bytes"),
        _ => base,
    }
}

/// State preloaded with RA, RB, and VS.
fn state_with_ra_rb_vs(
    ra_index: usize,
    ra_value: u64,
    rb_index: usize,
    rb_value: u64,
    vs_index: usize,
    vs_value: u128,
) -> PpuStateSnapshot {
    let mut s = PpuStateSnapshot::zero();
    s.gpr[ra_index] = ra_value;
    s.gpr[rb_index] = rb_value;
    s.vr[vs_index] = vs_value;
    s
}

/// Spec-derived cases for the entire Cell-unaligned VXU store family.
pub fn cases() -> Vec<InstructionCase> {
    let mut v = Vec::new();
    v.extend(stvlx_cases());
    v.extend(stvrx_cases());
    v.extend(stvlxl_cases());
    v.extend(stvrxl_cases());
    v
}

const VS_PATTERN: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;

/// `stvlx`: write the high `16 - m` bytes of VS starting at EA.
// [CBE-Handbook p:744 s:A.3.3] stvlx VS, RA, RB.
fn stvlx_cases() -> Vec<InstructionCase> {
    let raw = xform(VS_IDX_STVLX, RA_IDX, RB_IDX, XO_STVLX);
    stvl_cases_for_xo(raw, "stvlx", VS_PATTERN, VS_IDX_STVLX as usize)
}

/// `stvrx`: write the low `m` bytes of VS at the aligned line below EA.
// [CBE-Handbook p:744 s:A.3.3] stvrx VS, RA, RB.
fn stvrx_cases() -> Vec<InstructionCase> {
    let raw = xform(VS_IDX_STVRX, RA_IDX, RB_IDX, XO_STVRX);
    stvr_cases_for_xo(raw, "stvrx", VS_PATTERN, VS_IDX_STVRX as usize)
}

/// `stvlxl`: identical to stvlx + LRU hint.
fn stvlxl_cases() -> Vec<InstructionCase> {
    let raw = xform(VS_IDX_STVLXL, RA_IDX, RB_IDX, XO_STVLXL);
    stvl_cases_for_xo(raw, "stvlxl", VS_PATTERN, VS_IDX_STVLXL as usize)
}

/// `stvrxl`: identical to stvrx + LRU hint.
fn stvrxl_cases() -> Vec<InstructionCase> {
    let raw = xform(VS_IDX_STVRXL, RA_IDX, RB_IDX, XO_STVRXL);
    stvr_cases_for_xo(raw, "stvrxl", VS_PATTERN, VS_IDX_STVRXL as usize)
}

/// Shared case generator for stvlx / stvlxl. Covers EA offsets 0,
/// 4, 8, and 15 within the page -- the first three exercise common
/// alignments and the last tests the smallest possible count (1
/// byte).
fn stvl_cases_for_xo(
    raw: u32,
    prefix: &'static str,
    vs_pattern: u128,
    vs_index: usize,
) -> Vec<InstructionCase> {
    let vs_bytes = vs_pattern.to_be_bytes();
    let mut cases = Vec::new();
    for ea_offset in [0u64, 4, 8, 15] {
        let initial = state_with_ra_rb_vs(
            RA_IDX as usize,
            BASE_ADDR,
            RB_IDX as usize,
            ea_offset,
            vs_index,
            vs_pattern,
        );
        // Stores have no register side effects; only memory changes.
        let expected_state = initial.clone();
        let (_, count, _) = ea_decompose(ea_offset);
        let mut expected_bytes = vec![0u8; 32];
        expected_bytes[ea_offset as usize..ea_offset as usize + count]
            .copy_from_slice(&vs_bytes[..count]);
        cases.push(case_keep_memory(
            case_label(prefix, ea_offset, count),
            raw,
            initial,
            zero_memory(32),
            expected_state,
            memory_with(expected_bytes),
            "[CBE-Handbook p:744 s:A.3.3] stvlx: VS[0..16-m] -> MEM(EA, 16-m); m=EA&0xF",
        ));
    }
    cases
}

/// Shared case generator for stvrx / stvrxl. EA offsets 0, 1, 8,
/// and 15 cover zero bytes (aligned EA), single byte, half line,
/// and 15-byte tail within the first aligned line; 17, 24, and 31
/// repeat those shapes in the second line, so the aligned base
/// below EA is non-zero.
fn stvr_cases_for_xo(
    raw: u32,
    prefix: &'static str,
    vs_pattern: u128,
    vs_index: usize,
) -> Vec<InstructionCase> {
    let vs_bytes = vs_pattern.to_be_bytes();
    let mut cases = Vec::new();
    for ea_offset in [0u64, 1, 8, 15, 17, 24, 31] {
        let initial = state_with_ra_rb_vs(
            RA_IDX as usize,
            BASE_ADDR,
            RB_IDX as usize,
            ea_offset,
            vs_index,
            vs_pattern,
        );
        // Stores have no register side effects; only memory changes.
        let expected_state = initial.clone();
        let (m, _, aligned_offset) = ea_decompose(ea_offset);
        let mut expected_bytes = vec![0u8; 48];
        for i in 0..m {
            expected_bytes[aligned_offset + i] = vs_bytes[16 - m + i];
        }
        cases.push(case_keep_memory(
            case_label(prefix, ea_offset, m),
            raw,
            initial,
            zero_memory(48),
            expected_state,
            memory_with(expected_bytes),
            "[CBE-Handbook p:744 s:A.3.3] stvrx: VS[16-m..16] -> MEM(EA&~0xF, m); m=EA&0xF",
        ));
    }
    cases
}

#[cfg(test)]
#[path = "tests/cell_unaligned_vxu_stores_tests.rs"]
mod tests;
