#!/usr/bin/env bash
# The cargo-fuzz jobs for the ELF and PRX parsers: nightly-only, so they
# run on the scheduled loader-fuzz workflow and by hand, never in the
# continuous build. The bounded sample of the same property runs on
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

# The committed seeds start every target's corpus; libFuzzer adds what
# it finds beside them, and the workflow caches the directory between
# runs.
seed() {
    local target=$1
    mkdir -p "fuzz/corpus/$target"
    cp fuzz/seeds/*.bin "fuzz/corpus/$target/"
}

build() {
    cargo "+$toolchain" fuzz build --fuzz-dir fuzz -s "$sanitizer" "$@"
}

run() {
    local target=$1
    seed "$target"
    cargo "+$toolchain" fuzz run --fuzz-dir fuzz -s "$sanitizer" "$target" -- \
        "-max_total_time=$seconds" "-timeout=$timeout_s" "-rss_limit_mb=$rss_mb"
}

case "${1:-}" in
    seed) seed "${2:?target}" ;;
    build) shift; build "$@" ;;
    run) run "${2:?target}" ;;
    *)
        echo "usage: $0 {seed <target>|build [args]|run <target>}" >&2
        exit 2
        ;;
esac
