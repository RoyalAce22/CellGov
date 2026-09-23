#!/usr/bin/env bash
# Shared command groups for GitHub Actions and the local pre-push gate.

set -euo pipefail

external_data_features="${EXTERNAL_DATA_FEATURES:-$(sed -n '/^[[:space:]]*EXTERNAL_DATA_FEATURES: >-/{n;p;}' .github/workflows/ci.yml | tr -d '[:space:]')}"
test -n "$external_data_features"

lint() {
    export RUSTFLAGS='-D warnings'
    cargo fmt --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo clippy --workspace --all-targets --locked --features "$external_data_features" -- -D warnings
    cargo clippy -p cellgov_compare --all-targets --locked --no-default-features -- -D warnings
    RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
}

test_suite() {
    cargo check --workspace --all-targets --locked --features "$external_data_features"
    cargo test --workspace --locked
    cargo test --workspace --release --locked
    # The bounded fuzz smoke set, in both profiles: a debug invariant a raw
    # word trips is a finding only in the debug build. Every finding is
    # held to a promoted regression; an unpromoted one fails the build with
    # its artifact and exact replay printed.
    #
    # The artifacts directory starts empty. CI restores `target` from its
    # cache, and a stored artifact refuses a later finding at the same path
    # whose evidence differs, which would fail the set for a stale file
    # rather than for the run.
    rm -rf target/fuzz-smoke
    cargo run -p cellgov_cli --locked -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/debug --regressions crates/cellgov_fuzz/regressions
    cargo run -p cellgov_cli --locked --release -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/release --regressions crates/cellgov_fuzz/regressions
    cargo test -p cellgov_install --locked --features decrypt
    cargo test -p cellgov_install --release --locked --features decrypt
    cargo test -p cellgov_compare --locked --no-default-features
    cargo bench --workspace --no-run --benches --locked
}

deny() {
    cargo deny check advisories bans licenses sources
}

case "${1:-full}" in
    lint) lint ;;
    test) test_suite ;;
    deny) deny ;;
    full)
        lint
        deny
        test_suite
        ;;
    *)
        echo "usage: $0 {lint|test|deny|full}" >&2
        exit 2
        ;;
esac
