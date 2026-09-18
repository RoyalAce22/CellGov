#!/usr/bin/env bash
# Shared command groups for GitHub Actions and the local pre-push gate.

set -euo pipefail

corpus_features="${CORPUS_FEATURES:-$(sed -n '/^[[:space:]]*CORPUS_FEATURES: >-/{n;p;}' .github/workflows/ci.yml | tr -d '[:space:]')}"
test -n "$corpus_features"

lint() {
    export RUSTFLAGS='-D warnings'
    cargo fmt --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo clippy --workspace --all-targets --locked --features "$corpus_features" -- -D warnings
    cargo clippy -p cellgov_compare --all-targets --locked --no-default-features -- -D warnings
    RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
}

test_suite() {
    cargo check --workspace --all-targets --locked --features "$corpus_features"
    cargo test --workspace --locked
    cargo test --workspace --release --locked
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
