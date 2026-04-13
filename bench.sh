#!/usr/bin/env bash
# CellGov benchmark harness.
#
# Usage:
#   ./bench.sh                  Run all benchmarks
#   ./bench.sh save <name>      Run and save baseline 
#   ./bench.sh compare <name>   Run and compare against saved baseline
#   ./bench.sh quick            Run only fast benchmarks (skip 260MB hash)
#
# Baselines are stored in target/criterion/ by criterion automatically.

set -euo pipefail

BENCHES=(
    "cellgov_ppu:ppu_bench"
    "cellgov_mem:mem_bench"
    "cellgov_core:core_bench"
)

SKIP_PATTERN="260mb"

run_bench() {
    local extra_args=("$@")
    for entry in "${BENCHES[@]}"; do
        local crate="${entry%%:*}"
        local bench="${entry##*:}"
        echo "--- $crate / $bench ---"
        cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]}" 2>&1
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
            # Run only the fast mem benchmarks (criterion filters by regex match)
            cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]}" "(content_hash/(1mb|16mb)|fnv1a|apply_commit)" 2>&1
        else
            cargo bench -p "$crate" --bench "$bench" -- "${extra_args[@]}" 2>&1
        fi
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
        echo "Running fast benchmarks (skipping 260MB hash)"
        echo
        run_bench_quick
        ;;
    "")
        echo "Running all benchmarks"
        echo
        run_bench
        ;;
    *)
        echo "usage: bench.sh [save <name> | compare <name> | quick]"
        exit 1
        ;;
esac
