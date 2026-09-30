#!/usr/bin/env bash
# Shared command groups for GitHub Actions and the local pre-push gate.
#
# Every command prints one timing line, `ci.sh: <group>/<command> <seconds>s`,
# and every group one more, `ci.sh: <group> <seconds>s`. They are the record
# of where a run spends its time. Under GitHub Actions the same lines are
# also written to the step summary.
#
# The workflow runs these groups with CARGO_PROFILE_DEV_DEBUG=0, and
# rust-cache sets CARGO_INCREMENTAL=0. Neither changes what a command
# checks. Apart from the toolchain each job installs, they are the only
# differences from a local run.

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

# Every test group runs cargo under a feature and package selection of its
# own, and each distinct selection recompiles the crates whose features
# change. So the groups the workflow runs on both platforms hold three
# compile passes, and the rest run where they cost least:
#
#   test        both platforms: the external-data check, the debug
#               workspace tests (the decrypt-off build the README
#               promises), and the release workspace tests with
#               `cellgov_cli/decrypt` (the key-consuming paths, and every
#               `debug_assert!`-free path the release build compiles)
#   test-linux  the Linux stable leg: builds whose result does not depend
#               on the platform
#   local       the pre-push gate only: the decrypt-off release tests and
#               the decrypt-on debug tests of the two crates that declare
#               the feature, so every pairing of profile and `decrypt`
#               still runs before a push
#
# The bounded fuzz smoke set runs inside the workspace tests, in both
# profiles (`cli::fuzz::smoke_tests`), against the tracked regressions.
test_suite() {
    timed check-external-data cargo check --workspace --all-targets --locked --features "$external_data_features"
    timed test-debug cargo test --workspace --locked
    timed test-release-decrypt cargo test --workspace --release --locked --features cellgov_cli/decrypt
}

test_linux() {
    timed test-compare-no-default cargo test -p cellgov_compare --locked --no-default-features
    timed bench-build cargo bench --workspace --no-run --benches --locked
}

test_local() {
    timed test-release cargo test --workspace --release --locked
    timed test-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked --features cellgov_cli/decrypt
}

# The smoke set's finding artifacts, written where the workflow uploads
# them after a red test step. The set is deterministic, so this run
# stores what the failing test found; the test's scratch directory is
# gone by then. The directory starts empty, because a stored artifact
# refuses a later finding at the same path whose evidence differs.
smoke_artifacts() {
    rm -rf target/fuzz-smoke
    timed fuzz-smoke-debug cargo run -p cellgov_cli --locked -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/debug --regressions crates/cellgov_fuzz/regressions
    timed fuzz-smoke-release cargo run -p cellgov_cli --locked --release -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/release --regressions crates/cellgov_fuzz/regressions
}

# The MSRV promise is that the workspace builds on it; the workflow's
# msrv job runs this group on the MSRV toolchain, and the pre-push hook
# runs it the same way. Tests run on the pinned and stable toolchains.
check() {
    timed check cargo check --workspace --all-targets --locked
    timed check-external-data cargo check --workspace --all-targets --locked --features "$external_data_features"
}

deny() {
    timed deny cargo deny check advisories bans licenses sources
}

case "${1:-full}" in
    lint) run_group lint lint ;;
    check) run_group check check ;;
    test) run_group test test_suite ;;
    test-linux) run_group test-linux test_linux ;;
    local) run_group local test_local ;;
    smoke-artifacts) run_group smoke-artifacts smoke_artifacts ;;
    deny) run_group deny deny ;;
    # The pre-push gate: every group the workflow runs, plus `local`.
    full)
        run_group lint lint
        run_group deny deny
        run_group test test_suite
        run_group test-linux test_linux
        run_group local test_local
        ;;
    *)
        echo "usage: $0 {lint|check|test|test-linux|local|smoke-artifacts|deny|full}" >&2
        exit 2
        ;;
esac
