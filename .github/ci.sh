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
# checks. Apart from the toolchain each job installs and the test runner
# the blocking test legs select (see test_suite), they are the only
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
#
# CELLGOV_TEST_RUNNER=nextest (the workflow's blocking test legs) runs the
# test binaries through cargo-nextest's `ci` and `ci-release` profiles
# (.config/nextest.toml): a per-test timeout and a JUnit record of every
# test's duration. nextest does not run doctests, so a `cargo test --doc`
# run replaces them. Unset, the group runs plain `cargo test`.
test_suite() {
    timed check-external-data cargo check --workspace --all-targets --locked --features "$external_data_features"
    if [ "${CELLGOV_TEST_RUNNER:-cargo}" = nextest ]; then
        timed test-debug cargo nextest run --workspace --locked --profile ci
        timed test-doc cargo test --workspace --locked --doc
        timed test-release-decrypt cargo nextest run --workspace --release --locked \
            --features cellgov_cli/decrypt --profile ci-release
    else
        timed test-debug cargo test --workspace --locked
        timed test-release-decrypt cargo test --workspace --release --locked --features cellgov_cli/decrypt
    fi
}

test_linux() {
    timed test-compare-no-default cargo test -p cellgov_compare --locked --no-default-features
    timed bench-build cargo bench --workspace --no-run --benches --locked
    # The microtest post-link patcher, on ELF images built in memory.
    # -B writes no __pycache__ into the tree.
    timed patch-toc python3 -B -m unittest discover -s tests/micro/common -p 'test_*.py'
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
        --artifacts-dir target/fuzz-smoke/debug --regressions crates/cellgov_fuzz/regressions \
        --spu-references crates/cellgov_fuzz/tests/fixtures/spu_reference
    timed fuzz-smoke-release cargo run -p cellgov_cli --locked --release -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/release --regressions crates/cellgov_fuzz/regressions \
        --spu-references crates/cellgov_fuzz/tests/fixtures/spu_reference
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

# The microtest suites read the ELFs each tests/micro/<name>/build.sh
# writes under its build/ directory, which git ignores. The workflow's
# microtests job runs `micro-build` in the pinned toolchain image
# (tests/micro/toolchain) when its cache of those directories misses,
# then `microtests`. A build's output goes to target/micro-build/<name>.log
# and is printed only when the build fails.
build_every_microtest() {
    local image=$1 root dir name built=0
    # Docker on Windows takes a drive path; Git Bash would rewrite the
    # container paths.
    root=$(pwd -W 2>/dev/null || pwd)
    mkdir -p target/micro-build
    for dir in tests/micro/*/; do
        name=$(basename "$dir")
        [ -f "$dir/build.sh" ] || continue
        if ! MSYS_NO_PATHCONV=1 docker run --rm \
            -v "$root/tests/micro/$name:/src" -v "$root/tests/micro/common:/common" \
            -e COMMON=/common "$image" bash /src/build.sh >"target/micro-build/$name.log" 2>&1; then
            cat "target/micro-build/$name.log"
            echo "ci.sh: tests/micro/$name/build.sh failed" >&2
            return 1
        fi
        built=$((built + 1))
    done
    echo "ci.sh: built $built microtests"
    test "$built" -gt 0
}

micro_build() {
    local image=${MICRO_IMAGE:-ps3dev}
    timed toolchain-image docker build -q -t "$image" tests/micro/toolchain
    timed build-microtests build_every_microtest "$image"
}

# The targets that hold a microtest-gated test, with the features on: the
# two interpreter crates' unit tests, the SPU float-edge program, the
# CLI's exploration of the ELF microtests, and the CLI's boot suite. The spawn suite, the SPURS suite and the boot cases
# that load a firmware or read the key vault need the operator's vfs/,
# so they run only under `installed-firmware-tests`, which this job
# leaves off.
microtests() {
    timed test-microtests cargo nextest run --locked --profile ci-microtests \
        -p cellgov_ppu -p cellgov_spu -p cellgov_cli \
        --features cellgov_ppu/ppu-microtests,cellgov_spu/spu-microtests,cellgov_cli/microtests \
        -E 'package(cellgov_ppu) & kind(lib) | package(cellgov_spu) & (kind(lib) | test(/^spu_float_edges::/)) | binary_id(cellgov_cli::microtests) | binary_id(cellgov_cli::bin/cellgov) & test(=cli::scenarios::tests::explore_micro_runs_for_elf_microtests)'
}

# The data-feature suites, on the operator's machine: every target and
# test behind an external-data feature, through nextest's `ci-data`
# profile, which records each test's duration in
# target/nextest/ci-data/junit.xml. The live-console suite
# (`runner_ps3/ps3-hardware`) is left out: it drives the PS3, so it runs
# only when the operator asks for that.
#
# Each run appends one line to the operator's ledger,
# vfs/.cellgov/data-runs.tsv (CELLGOV_DATA_LEDGER moves it): the UTC
# time, the commit, the installed firmware and title set, and nextest's
# summary. The line is written whether the run passed or failed, and the
# group then exits with nextest's status.
data_features=${external_data_features//,runner_ps3\/ps3-hardware/}
data_targets="firmware_set_load lv2_census_firmware ps3autotests process_spawn_microtest microtests spurs_microtest title_witnesses obs_null_sink authority_id bench_stdout_purity system_ver_floor"
install_data_targets="npdrm_oracle_vectors parity_pkg parity_disc parity_title_digests firmware_pipeline_end_to_end parity_firmware_decrypt"

# The installed firmware count and newest version, and the installed
# title ids, from `cellgov status`.
store_summary() {
    cargo run -q -p cellgov_cli --locked -- status --no-color --no-progress 2>/dev/null | awk '
        /^firmware/ { section = "fw" }
        /^titles/   { section = "titles" }
        /^anchors/  { section = "" }
        section == "fw" && NF {
            version = ($1 == "firmware") ? $2 : $1
            count++
            newest = version
        }
        section == "titles" && NF {
            id = ($1 == "titles") ? $2 : $1
            if ($0 !~ /nothing installed/) titles = titles (titles == "" ? "" : ",") id
        }
        END { printf "firmware=%d newest=%s\ttitles=%s\n", count, newest, titles }'
}

data_suites() {
    local filter target ledger log commit status=0
    filter='package(cellgov_ppu) & kind(lib) | package(cellgov_spu) & (kind(lib) | test(/^spu_float_edges::/))'
    filter+=' | binary_id(cellgov_cli::bin/cellgov) & test(=cli::scenarios::tests::explore_micro_runs_for_elf_microtests)'
    filter+=' | package(cellgov_install) & kind(lib) & test(/^sce::tests::/)'
    for target in $data_targets; do
        filter+=" | binary_id(cellgov_cli::$target)"
    done
    for target in $install_data_targets; do
        filter+=" | binary_id(cellgov_install::$target)"
    done
    ledger=${CELLGOV_DATA_LEDGER:-vfs/.cellgov/data-runs.tsv}
    log=target/nextest/ci-data.log
    mkdir -p target/nextest "$(dirname "$ledger")"
    commit=$(git rev-parse --short=12 HEAD)
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        commit="$commit+dirty"
    fi
    cargo nextest run --locked --profile ci-data \
        -p cellgov_ppu -p cellgov_spu -p cellgov_cli -p cellgov_install \
        --features "$data_features" -E "$filter" 2>&1 | tee "$log" || status=$?
    [ -f "$ledger" ] || printf 'utc\tcommit\tfirmware\ttitles\tnextest\n' >"$ledger"
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$(store_summary)" \
        "$(sed -n 's/^ *Summary \[[^]]*\] *//p' "$log" | tail -1)" >>"$ledger"
    echo "ci.sh: data run recorded in $ledger"
    return "$status"
}

data() {
    timed data-suites data_suites
}

case "${1:-full}" in
    lint) run_group lint lint ;;
    check) run_group check check ;;
    test) run_group test test_suite ;;
    test-linux) run_group test-linux test_linux ;;
    local) run_group local test_local ;;
    smoke-artifacts) run_group smoke-artifacts smoke_artifacts ;;
    deny) run_group deny deny ;;
    micro-build) run_group micro-build micro_build ;;
    microtests) run_group microtests microtests ;;
    data) run_group data data ;;
    # The pre-push gate: every group the workflow runs, plus `local`.
    full)
        run_group lint lint
        run_group deny deny
        run_group test test_suite
        run_group test-linux test_linux
        run_group local test_local
        ;;
    *)
        echo "usage: $0 {lint|check|test|test-linux|local|smoke-artifacts|deny|micro-build|microtests|data|full}" >&2
        exit 2
        ;;
esac
