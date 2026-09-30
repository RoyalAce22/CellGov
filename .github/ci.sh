#!/usr/bin/env bash
# Shared command groups for GitHub Actions and the local pre-push gate.
#
# Every command prints one timing line, `ci.sh: <group>/<command> <seconds>s`,
# and every group one more, `ci.sh: <group> <seconds>s`. They are the record
# of where a run spends its time. Under GitHub Actions the same lines are
# also written to the step summary.

set -euo pipefail

external_data_features="${EXTERNAL_DATA_FEATURES:-$(sed -n '/^[[:space:]]*EXTERNAL_DATA_FEATURES: >-/{n;p;}' .github/workflows/ci.yml | tr -d '[:space:]')}"
test -n "$external_data_features"

summary() {
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        printf '%s\n' "$1" >>"$GITHUB_STEP_SUMMARY"
    fi
}

group_line() {
    echo "ci.sh: $group $((SECONDS - group_start))s$1"
    summary "| **$group** | **$((SECONDS - group_start))** | $2 |"
}

# timed <label> <command...>: runs one command and prints its wall time.
# A failing command prints its line and its group's line, then ends the
# script with the command's status, as `set -e` would.
timed() {
    local label=$1 start=$SECONDS status=0
    shift
    "$@" || status=$?
    if [ "$status" -eq 0 ]; then
        echo "ci.sh: $group/$label $((SECONDS - start))s"
    else
        echo "ci.sh: $group/$label $((SECONDS - start))s (exit $status)"
    fi
    summary "| \`$group/$label\` | $((SECONDS - start)) | $status |"
    if [ "$status" -ne 0 ]; then
        group_line " (failed)" "$status"
        exit "$status"
    fi
}

# run_group <name> <function>: runs one command group under a timing table.
# The function runs under `set -e`; it is never the operand of `||` or
# `if`, which would switch that off for every command inside it.
run_group() {
    group=$1
    group_start=$SECONDS
    summary ""
    summary "| \`ci.sh\` command | seconds | exit |"
    summary "|---|---|---|"
    "$2"
    group_line "" 0
}

lint() {
    export RUSTFLAGS='-D warnings'
    timed fmt cargo fmt --check
    timed clippy cargo clippy --workspace --all-targets --locked -- -D warnings
    timed clippy-external-data cargo clippy --workspace --all-targets --locked --features "$external_data_features" -- -D warnings
    timed clippy-compare-no-default cargo clippy -p cellgov_compare --all-targets --locked --no-default-features -- -D warnings
    # Private items too: a broken link in a private doc comment is as
    # misleading as a public one, and the public build never resolves it.
    timed doc env RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --document-private-items --locked
    timed doc-external-data env RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --document-private-items --locked --features "$external_data_features"
}

test_suite() {
    timed check-external-data cargo check --workspace --all-targets --locked --features "$external_data_features"
    timed test-debug cargo test --workspace --locked
    timed test-release cargo test --workspace --release --locked
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
    timed fuzz-smoke-debug cargo run -p cellgov_cli --locked -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/debug --regressions crates/cellgov_fuzz/regressions
    timed fuzz-smoke-release cargo run -p cellgov_cli --locked --release -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/release --regressions crates/cellgov_fuzz/regressions
    timed test-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked --features cellgov_cli/decrypt
    timed test-decrypt-release cargo test -p cellgov_cli -p cellgov_install --release --locked --features cellgov_cli/decrypt
    timed test-compare-no-default cargo test -p cellgov_compare --locked --no-default-features
    timed bench-build cargo bench --workspace --no-run --benches --locked
}

deny() {
    timed deny cargo deny check advisories bans licenses sources
}

case "${1:-full}" in
    lint) run_group lint lint ;;
    test) run_group test test_suite ;;
    deny) run_group deny deny ;;
    full)
        run_group lint lint
        run_group deny deny
        run_group test test_suite
        ;;
    *)
        echo "usage: $0 {lint|test|deny|full}" >&2
        exit 2
        ;;
esac
