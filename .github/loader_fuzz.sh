#!/usr/bin/env bash
# The cargo-fuzz jobs for the ELF, PRX and SPU image parsers:
# nightly-only, so they run on the scheduled loader-fuzz workflow and by
# hand, never in the continuous build. The bounded sample of the same property runs on
# stable inside `cargo test -p cellgov_fuzz`.

set -euo pipefail

seconds="${FUZZ_SECONDS:-900}"
timeout_s="${FUZZ_TIMEOUT:-10}"
rss_mb="${FUZZ_RSS_MB:-2048}"
toolchain="${FUZZ_TOOLCHAIN:-nightly}"
# AddressSanitizer by default; `none` trades its memory checks for
# speed. A Windows host runs the sanitized binaries only with the MSVC
# toolchain's `clang_rt.asan_dynamic-x86_64.dll` on the path.
sanitizer="${FUZZ_SANITIZER:-address}"

# The committed seeds start every target's input set; libFuzzer adds
# what it finds beside them, and the workflow keeps the directory
# between runs as an artifact.
seed() {
    local target=$1
    mkdir -p "fuzz/inputs/$target"
    cp fuzz/seeds/*.bin "fuzz/inputs/$target/"
}

count() {
    find "fuzz/inputs/$1" -type f 2>/dev/null | wc -l | tr -d '[:space:]'
}

# The workflow's restore: the newest unexpired `loader-fuzz-inputs-<target>`
# artifact, unpacked into the target's input directory. Having none yet is
# not an error; the seeds start the set. Needs `gh` with `actions: read`
# and GITHUB_REPOSITORY.
restore() {
    local target=$1 name="loader-fuzz-inputs-$1" run_id
    run_id=$(gh api "repos/$GITHUB_REPOSITORY/actions/artifacts?name=$name&per_page=20" \
        --jq '[.artifacts[] | select(.expired | not)] | sort_by(.created_at) | last | .workflow_run.id // empty')
    if [ -z "$run_id" ]; then
        echo "loader-fuzz: $target: no earlier inputs artifact; the seeds start the set"
        return 0
    fi
    mkdir -p "fuzz/inputs/$target"
    gh run download "$run_id" --repo "$GITHUB_REPOSITORY" --name "$name" --dir "fuzz/inputs/$target"
    echo "loader-fuzz: $target: restored $(count "$target") inputs from run $run_id"
}

build() {
    cargo "+$toolchain" fuzz build --fuzz-dir fuzz -s "$sanitizer" "$@"
}

# Prints the input count before and after, so two runs in a row show
# whether the second started from the first one's grown set.
run() {
    local target=$1 status=0
    seed "$target"
    echo "loader-fuzz: $target: $(count "$target") inputs at start"
    cargo "+$toolchain" fuzz run --fuzz-dir fuzz -s "$sanitizer" "$target" "fuzz/inputs/$target" -- \
        "-max_total_time=$seconds" "-timeout=$timeout_s" "-rss_limit_mb=$rss_mb" || status=$?
    echo "loader-fuzz: $target: $(count "$target") inputs at end"
    return "$status"
}

# Names a run's outcome from the fuzz step's result and the files libFuzzer
# left under fuzz/artifacts/<target>: a crash, a timeout or out-of-memory
# input, or clean. The prefixes are libFuzzer's own artifact names.
outcome() {
    local target=$1 result=$2 dir="fuzz/artifacts/$1" kind file
    case "$result" in
        success)
            echo "loader-fuzz $target: clean"
            return 0
            ;;
        failure) ;;
        *)
            echo "loader-fuzz $target: did not finish (${result:-not run})"
            return 0
            ;;
    esac
    for kind in crash leak timeout oom slow-unit; do
        for file in "$dir/$kind-"*; do
            [ -e "$file" ] || continue
            case "$kind" in
                crash | leak) echo "loader-fuzz $target: crash, input $(basename "$file") in artifact loader-fuzz-$target" ;;
                *) echo "loader-fuzz $target: timeout/out-of-memory, input $(basename "$file") in artifact loader-fuzz-$target" ;;
            esac
            return 0
        done
    done
    echo "loader-fuzz $target: failed with no libFuzzer artifact (a build or toolchain failure; see the log)"
}

case "${1:-}" in
    seed) seed "${2:?target}" ;;
    restore) restore "${2:?target}" ;;
    build) shift; build "$@" ;;
    run) run "${2:?target}" ;;
    outcome) outcome "${2:?target}" "${3:-}" ;;
    *)
        echo "usage: $0 {seed <target>|restore <target>|build [args]|run <target>|outcome <target> <step result>}" >&2
        exit 2
        ;;
esac
