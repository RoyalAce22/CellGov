#!/usr/bin/env bash
# CellGov benchmark harness.
#
# Usage:
#   ./bench.sh                  Run all benchmarks
#   ./bench.sh save <name>      Run and save baseline
#   ./bench.sh compare <name>   Run and compare against saved baseline
#   ./bench.sh quick            Run only fast benchmarks (skip 260 MB and 1 GB)
#   ./bench.sh list             List bench IDs without running them
#
# Baselines are stored in target/criterion/ by criterion automatically.
#
# Requires bash >= 4.0 (associative-array / empty-array semantics).
# macOS ships bash 3.2 by default; `brew install bash` for a modern one.
#
# 'compare' mode assumes baselines were saved on the same machine,
# same toolchain, and same bench-ID set. Cross-machine compare is
# not supported -- criterion does not embed provenance, and a
# compare across a toolchain or CPU change renders meaningless drift
# numbers rather than refusing or warning.

set -euo pipefail

BENCHES=(
    "cellgov_ppu:ppu_bench"
    "cellgov_mem:mem_bench"
    "cellgov_core:core_bench"
    "cellgov_compare:diverge_bench"
)

# Anchored alternation regex for `quick` mode. The `^...$` anchoring is
# load-bearing: criterion treats the filter as `Regex::is_match` on the
# full bench id and does NOT insert anchors, so a bare alternation
# `content_hash/1mb|...` substring-matches `content_hash/1mb_cached`
# and overshoots. Each branch must be a full ID for the filter to be
# exact. Renames in this list are caught by the
# `cargo bench --no-run --benches` CI gate before they reach a quick
# run; a renamed ID would compile fine but match zero benches, and
# criterion exits non-zero only on "filter matched zero overall," not
# on "this branch matched nothing" -- so per-branch validation is a
# follow-up if silent rename starts firing in practice.
MEM_QUICK_FILTER='^(content_hash/1mb_dirty|content_hash/16mb_dirty|fnv1a_raw/1mb|fnv1a_raw/16mb|apply_commit/4b|apply_commit/4kb|apply_commit/main_region/4b|apply_commit/stack_region/8b)$'

run_bench() {
    local extra_args=("$@")
    for entry in "${BENCHES[@]}"; do
        local crate="${entry%%:*}"
        local bench="${entry##*:}"
        echo "--- $crate / $bench ---"
        # The "${arr[@]+"${arr[@]}"}" form guards against `set -u` failure
        # when arr is empty (bash 3.2 expands "${arr[@]}" to an unbound
        # error; the +alternate form yields nothing instead).
        cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]+"${extra_args[@]}"}" 2>&1
        echo
    done
}

run_bench_quick() {
    local extra_args=("$@")
    for entry in "${BENCHES[@]}"; do
        local crate="${entry%%:*}"
        local bench="${entry##*:}"
        echo "--- $crate / $bench ---"
        if [ "$bench" = "mem_bench" ]; then
            cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]+"${extra_args[@]}"}" "$MEM_QUICK_FILTER" 2>&1
        else
            cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]+"${extra_args[@]}"}" 2>&1
        fi
        echo
    done
}

list_benches() {
    for entry in "${BENCHES[@]}"; do
        local crate="${entry%%:*}"
        local bench="${entry##*:}"
        echo "--- $crate / $bench ---"
        # criterion's --list prints the ID set without running. Cannot be
        # combined with --no-run: cargo intercepts --no-run and never
        # invokes the bench binary, so flags after `--` are dropped.
        cargo bench -p "$crate" --bench "$bench" -- --list 2>&1
        echo
    done
}

case "${1:-}" in
    save)
        name="${2:?usage: bench.sh save <name>}"
        echo "Saving baseline: $name"
        echo
        run_bench --save-baseline "$name"
        echo "Baseline '$name' saved to target/criterion/"
        ;;
    compare)
        name="${2:?usage: bench.sh compare <name>}"
        echo "Comparing against baseline: $name"
        echo
        run_bench --baseline "$name"
        ;;
    quick)
        echo "Running fast benchmarks (skipping 260 MB and 1 GB tail)"
        echo
        run_bench_quick
        ;;
    list)
        list_benches
        ;;
    "")
        echo "Running all benchmarks"
        echo
        run_bench
        ;;
    *)
        echo "usage: bench.sh [save <name> | compare <name> | quick | list]"
        exit 1
        ;;
esac
