#!/usr/bin/env bash
# The scheduled fuzz jobs: the long runs a continuous build never pays for.
# Every command here reads the same library, result and artifact contracts
# as the bounded smoke set in ci.sh; only the budgets differ.

set -euo pipefail

cases="${FUZZ_CASES:-20000}"
trials="${FUZZ_TRIALS:-10}"
raw_words="${FUZZ_RAW_WORDS:-16777216}"
out="${FUZZ_OUT:-target/fuzz-scheduled}"
regressions=crates/cellgov_fuzz/regressions

cellgov() {
    cargo run --release -p cellgov_cli --locked -- "$@"
}

# Each job's output starts empty. The workflow restores `target` from its
# cache, and a stored artifact refuses a later finding at the same path
# whose evidence differs; a stale result would also upload as this run's.
fresh() {
    rm -rf "$1"
    mkdir -p "$1"
}

campaigns() {
    fresh "$out/campaigns"
    for engine in ppu-instruction ppu-sequence spu-instruction spu-sequence; do
        for strategy in structured raw-words; do
            cellgov dev fuzz "$engine" --strategy "$strategy" --seed 1 --count "$cases" \
                --reduction on-finding --artifacts-dir "$out/campaigns/$engine-$strategy"
        done
    done
}

# A raw scan keeps its panic samples as scanned; the command refuses a
# reduction request as usage.
raw() {
    fresh "$out/raw"
    for decoder in ppu spu; do
        cellgov dev fuzz raw "$decoder" --start 0 --count "$raw_words" \
            --output "$out/raw/$decoder.json"
    done
}

evaluate() {
    fresh "$out/evaluation"
    for engine in ppu-instruction ppu-sequence spu-instruction spu-sequence; do
        cellgov dev fuzz evaluate "$engine" --trials "$trials" --cases $((cases / 10)) \
            --output "$out/evaluation/$engine.json"
    done
}

smoke() {
    fresh "$out/smoke"
    cellgov dev fuzz smoke --artifacts-dir "$out/smoke" --regressions "$regressions"
}

case "${1:-}" in
    campaigns) campaigns ;;
    raw) raw ;;
    evaluate) evaluate ;;
    smoke) smoke ;;
    *)
        echo "usage: $0 {campaigns|raw|evaluate|smoke}" >&2
        exit 2
        ;;
esac
