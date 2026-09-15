//! End-of-run tallies, reported to the caller's sink.

use cellgov_core::{AddressSpaceId, Runtime};

use super::{fetch_raw_at, format_hle_idx};
use crate::BootSink;

/// Which HLE imports the run called, and how often.
pub fn report_hle_summary(hle_calls: &std::collections::BTreeMap<u32, usize>, sink: &dyn BootSink) {
    let called_count = hle_calls.len();
    if called_count == 0 {
        return;
    }
    sink.note(&format!(
        "hle_imports: {called_count} called (no binder; routed to LV2 Unsupported)"
    ));
    for (idx, count) in hle_calls {
        sink.note(&format!("    {}: {count}x", format_hle_idx(*idx)));
    }
}

/// Which instruction variants the run retired, most frequent first.
pub fn report_insn_coverage(
    insn_coverage: &std::collections::BTreeMap<&'static str, usize>,
    sink: &dyn BootSink,
) {
    if insn_coverage.is_empty() {
        sink.note("instruction_coverage: none");
        return;
    }
    let mut sorted: Vec<_> = insn_coverage.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    sink.note(&format!(
        "instruction_coverage: {} variants executed",
        sorted.len()
    ));
    for (name, count) in &sorted {
        sink.note(&format!("  {name}: {count}x"));
    }
}

/// Predecode-shadow hit rate, per unit and in total.
///
/// A rising per-unit miss count means its fetches moved outside the
/// shadowed region (PRX bodies above 0x10000000).
pub fn report_shadow_stats(rt: &mut Runtime, sink: &dyn BootSink) {
    let mut per_unit: Vec<(u64, u64, u64)> = Vec::new();
    let mut total_hits = 0u64;
    let mut total_misses = 0u64;
    let mut total_units = 0usize;
    for (id, unit) in rt.units_mut() {
        total_units += 1;
        let (h, m) = unit.shadow_stats();
        if h + m == 0 {
            continue;
        }
        per_unit.push((id.raw(), h, m));
        total_hits += h;
        total_misses += m;
    }
    let total = total_hits + total_misses;
    if total == 0 {
        sink.note("shadow: no fetches recorded");
        return;
    }
    let hit_pct = (total_hits as f64 / total as f64) * 100.0;
    let active = per_unit.len();
    sink.note(&format!(
        "shadow: {total_hits}/{total} via shadow ({hit_pct:.1}%), {total_misses} decode-on-fetch ({active} active / {total_units} registered)"
    ));
    if active > 1 {
        for (unit_id, h, m) in &per_unit {
            let t = h + m;
            let pct = (*h as f64 / t as f64) * 100.0;
            sink.note(&format!(
                "  unit {unit_id}: {h}/{t} via shadow ({pct:.1}%), {m} decode-on-fetch"
            ));
        }
    }
}

/// The twenty hottest PCs of the run, with the instruction at each.
pub fn report_top_pcs(
    rt: &Runtime,
    pc_hits: &std::collections::BTreeMap<(AddressSpaceId, u64), u64>,
    sink: &dyn BootSink,
) {
    if pc_hits.is_empty() {
        return;
    }
    let mut sorted: Vec<_> = pc_hits.iter().collect();
    // Tie-break by (space, PC) so the ranking is independent of iteration order.
    sorted.sort_by(|&(key_a, c_a), &(key_b, c_b)| c_b.cmp(c_a).then(key_a.cmp(key_b)));
    sink.note("top_pcs_by_hit_count:");
    for (key, count) in sorted.iter().take(20) {
        let (space, pc) = **key;
        let (raw, disasm) = match rt.space_memory(space) {
            // A unit that stepped executes in a live space; a miss here
            // is runtime-state corruption and is named, not blanked.
            Err(e) => (
                format!("<space {} missing: {e}>", space.raw()),
                String::new(),
            ),
            Ok(mem) => match fetch_raw_at(mem, pc) {
                Some(w) => (
                    format!("0x{w:08x}"),
                    cellgov_ppu::decode::decode(w)
                        .ok()
                        .map(|insn| <&'static str>::from(&insn).to_string())
                        .unwrap_or_else(|| "<baddec>".into()),
                ),
                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
            },
        };
        let space_tag = if space == AddressSpaceId::BOOT {
            String::new()
        } else {
            format!("  space={}", space.raw())
        };
        sink.note(&format!(
            "  {count:>10}x  PC=0x{pc:08x}  raw={raw}  {disasm}{space_tag}"
        ));
    }
}
