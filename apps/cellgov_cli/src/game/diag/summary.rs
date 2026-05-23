use cellgov_core::Runtime;

use super::{fetch_raw_at, format_hle_idx};

pub(in crate::game) fn print_hle_summary(hle_calls: &std::collections::BTreeMap<u32, usize>) {
    let called_count = hle_calls.len();
    if called_count == 0 {
        return;
    }
    println!("hle_imports: {called_count} called (no binder; routed to LV2 Unsupported)");
    for (idx, count) in hle_calls {
        println!("    {}: {count}x", format_hle_idx(*idx));
    }
}

pub(in crate::game) fn print_insn_coverage(
    insn_coverage: &std::collections::BTreeMap<&'static str, usize>,
) {
    if insn_coverage.is_empty() {
        println!("instruction_coverage: none");
        return;
    }
    let mut sorted: Vec<_> = insn_coverage.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    println!("instruction_coverage: {} variants executed", sorted.len());
    for (name, count) in &sorted {
        println!("  {name}: {count}x");
    }
}

/// A rising per-unit miss count means its fetches moved outside the
/// shadowed region (PRX bodies above 0x10000000).
pub(in crate::game) fn print_shadow_stats(rt: &mut Runtime) {
    let mut per_unit: Vec<(u64, u64, u64)> = Vec::new();
    let mut total_hits = 0u64;
    let mut total_misses = 0u64;
    let mut total_units = 0usize;
    for (id, unit) in rt.registry_mut().iter_mut() {
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
        println!("shadow: no fetches recorded");
        return;
    }
    let hit_pct = (total_hits as f64 / total as f64) * 100.0;
    let active = per_unit.len();
    println!(
        "shadow: {total_hits}/{total} via shadow ({hit_pct:.1}%), {total_misses} decode-on-fetch ({active} active / {total_units} registered)"
    );
    if active > 1 {
        for (unit_id, h, m) in &per_unit {
            let t = h + m;
            let pct = (*h as f64 / t as f64) * 100.0;
            println!("  unit {unit_id}: {h}/{t} via shadow ({pct:.1}%), {m} decode-on-fetch");
        }
    }
}

pub(in crate::game) fn print_top_pcs(rt: &Runtime, pc_hits: &std::collections::BTreeMap<u64, u64>) {
    if pc_hits.is_empty() {
        return;
    }
    let mut sorted: Vec<_> = pc_hits.iter().collect();
    // Tie-break by PC so the ranking is independent of iteration order.
    sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
    println!("top_pcs_by_hit_count:");
    for (pc, count) in sorted.iter().take(20) {
        let (raw, disasm) = match fetch_raw_at(rt, **pc) {
            Some(w) => (
                format!("0x{w:08x}"),
                cellgov_ppu::decode::decode(w)
                    .ok()
                    .map(|insn| <&'static str>::from(&insn).to_string())
                    .unwrap_or_else(|| "<baddec>".into()),
            ),
            None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
        };
        println!("  {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
    }
}
