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

# Names a job's outcome from its exit status: clean, a timeout, killed
# (out of memory), or a finding with the files the uploaded artifact
# holds for it.
outcome() {
    local job=$1 status=$2 artifact=$3 files
    case "$status" in
        "") echo "fuzz $job: did not finish (no exit status recorded)" ;;
        0) echo "fuzz $job: clean" ;;
        124) echo "fuzz $job: timeout (the step's time budget ran out)" ;;
        127) echo "fuzz $job: did not start (exit 127, a command was not found)" ;;
        137) echo "fuzz $job: killed, most likely out of memory (exit 137)" ;;
        *)
            files=$(find "$out" -type f -name '*.json' 2>/dev/null | wc -l | tr -d '[:space:]')
            echo "fuzz $job: finding or refusal (exit $status); $files result file(s) in artifact $artifact"
            ;;
    esac
}

case "${1:-}" in
    outcome) outcome "${2:?job}" "${3:-}" "${4:?artifact name}" ;;
    campaigns) campaigns ;;
    raw) raw ;;
    evaluate) evaluate ;;
    smoke) smoke ;;
    *)
        echo "usage: $0 {campaigns|raw|evaluate|smoke|outcome <job> <status> <artifact>}" >&2
        exit 2
        ;;
esac
