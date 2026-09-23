//! Deterministic decoder-sweep partitions.

use crate::boundary::call_target;
use crate::TargetPanicPayload;
use std::ops::RangeInclusive;

/// One decoder panic captured at the target boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodePanic {
    /// Raw word passed to the decoder.
    pub raw: u32,
    /// Deterministic payload classification.
    pub payload: TargetPanicPayload,
}

/// Counts decoder results for one caller-selected partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeSweepReport {
    /// First raw word in the partition.
    pub first: u32,
    /// Last raw word in the partition.
    pub last: u32,
    /// Words accepted by the decoder.
    pub accepted: u64,
    /// Words refused by the decoder.
    pub refused: u64,
    /// Words that made the decoder panic.
    pub panics: Vec<DecodePanic>,
}

/// Checks the PPU decoder across the supplied word range.
pub fn ppu_decode_partition(words: RangeInclusive<u32>) -> DecodeSweepReport {
    run(words, |raw| cellgov_ppu::decode::decode(raw).is_ok())
}

/// Checks the SPU decoder across the supplied word range.
pub fn spu_decode_partition(words: RangeInclusive<u32>) -> DecodeSweepReport {
    run(words, |raw| cellgov_spu::decode::decode(raw).is_ok())
}

fn run(words: RangeInclusive<u32>, decode: impl Fn(u32) -> bool) -> DecodeSweepReport {
    let first = *words.start();
    let last = *words.end();
    let mut accepted = 0u64;
    let mut refused = 0u64;
    let mut panics = Vec::new();
    for raw in words {
        match call_target(|| decode(raw)) {
            Ok(true) => accepted += 1,
            Ok(false) => refused += 1,
            Err(payload) => panics.push(DecodePanic { raw, payload }),
        }
    }
    DecodeSweepReport {
        first,
        last,
        accepted,
        refused,
        panics,
    }
}
