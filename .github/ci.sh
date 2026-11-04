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
# differences from a local run, except that the pre-push gate runs the
# rustdoc lints through `rustdoc --check` (see rustdoc_lint).

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
#
# CELLGOV_TIMING_RECORD=<file> also appends one tab-separated row per
# command: `<group>/<label>`, seconds, exit status, the command.
timed() {
    local label=$1 start=$SECONDS status=0
    shift
    "$@" || status=$?
    if [ "$status" -eq 0 ]; then
        echo "ci.sh: $group/$label $((SECONDS - start))s"
    else
        echo "ci.sh: $group/$label $((SECONDS - start))s (exit $status)"
    fi
    if [ -n "${CELLGOV_TIMING_RECORD:-}" ]; then
        printf '%s\t%s\t%s\t%s\n' "$group/$label" "$((SECONDS - start))" "$status" "$*" >>"$CELLGOV_TIMING_RECORD"
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
    rustdoc_lint doc
    rustdoc_lint doc-external-data --features "$external_data_features"
}

# rustdoc_lint <label> [cargo doc args...]: the workspace rustdoc lints.
# CELLGOV_RUSTDOC_CHECK=1 (the pre-push gate) runs them through
# `rustdoc --check`, the same lint passes with no HTML written. The HTML
# build re-reads and re-merges the whole private-items search index for
# every crate it documents, most of a minute of one core each here, so
# a local run checks and the workflow keeps the full build. `--check` is
# unstable: RUSTC_BOOTSTRAP=1 enables it on the pinned toolchain for this
# command only, and a target dir of its own keeps those fingerprints
# apart from every other build.
rustdoc_lint() {
    local label=$1
    shift
    if [ "${CELLGOV_RUSTDOC_CHECK:-0}" = 1 ]; then
        timed "$label" env RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS='-D warnings -Z unstable-options --check' \
            CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}/rustdoc-check" \
            cargo doc --workspace --no-deps --document-private-items --locked "$@"
    else
        timed "$label" env RUSTDOCFLAGS='-D warnings' \
            cargo doc --workspace --no-deps --document-private-items --locked "$@"
    fi
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
#               still runs before a push. Both run plain `cargo test`,
#               which runs a binary's tests as threads of one process, so
#               a race over process-global state stays visible.
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

# The operator's per-branch gate, local only. `gate-tests` is the
# compiled-output checkpoint, `gate-docs` the closing proof when only
# comments and prose changed since it, and `gate-full` both. The
# commands are the workspace gate's: rustfmt, clippy with and without
# `cellgov_cli/decrypt`, the rustdoc lints through `rustdoc --check`
# (see rustdoc_lint), nextest's `ci` and `ci-release` profiles, the
# doctests, and plain `cargo test` of the two crates that declare
# `decrypt` in both profiles, which keeps a race over process-global
# state visible.
#
# The gate builds in target dirs of its own, so it never invalidates
# the operator's interactive builds and they never invalidate it. The
# debug lane builds with no debuginfo, the workflow's setting, and links
# with rust-lld (the variable is read on the MSVC target only): its
# test builds are link-bound, and after an edit to cellgov_core the
# workspace's took 12 s that way against 136 s with link.exe and full
# debuginfo. The release lane is compile-bound and builds as the
# workflow does, incremental compilation off: that changes how code is
# split for codegen, and with it which allocations LLVM removes, which
# cellgov_core's allocation_counts test reads.
#
# The two lanes build side by side, the release lane in a second target
# dir: its tail is a few crates at opt-level 3 that leave most cores
# idle. The tests run after both builds end, one suite at a time. Test
# processes started while the release build held 35 GB of commit on a
# 32 GB host stalled for 20 s and hit the per-test timeout.
gate_root=${CARGO_TARGET_DIR:-target}

gate_debug_env() {
    export CARGO_TARGET_DIR=$gate_root/gate-debug CARGO_PROFILE_DEV_DEBUG=0 \
        CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld
}

gate_release_env() {
    export CARGO_TARGET_DIR=$gate_root/gate-release
}

gate_debug_build() {
    gate_debug_env
    timed clippy cargo clippy --workspace --all-targets --locked -- -D warnings
    timed clippy-decrypt cargo clippy --workspace --all-targets --locked --features cellgov_cli/decrypt -- -D warnings
    if [ "$gate_mode" != tests ]; then
        CELLGOV_RUSTDOC_CHECK=1 rustdoc_lint doc
        CELLGOV_RUSTDOC_CHECK=1 rustdoc_lint doc-decrypt --features cellgov_cli/decrypt
    fi
    if [ "$gate_mode" != docs ]; then
        timed build-debug cargo nextest run --workspace --locked --profile ci --no-run
        timed build-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked \
            --features cellgov_cli/decrypt --no-run
    fi
}

gate_release_build() {
    gate_release_env
    timed build-release cargo nextest run --workspace --release --locked --profile ci-release --no-run
    timed build-decrypt-release cargo test -p cellgov_cli -p cellgov_install --release --locked \
        --features cellgov_cli/decrypt --no-run
}

gate_debug_tests() {
    gate_debug_env
    timed test-debug cargo nextest run --workspace --locked --profile ci
    timed test-doc cargo test --workspace --locked --doc
    timed test-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked --features cellgov_cli/decrypt
}

gate_release_tests() {
    gate_release_env
    timed test-release cargo nextest run --workspace --release --locked --profile ci-release
    timed test-decrypt-release cargo test -p cellgov_cli -p cellgov_install --release --locked \
        --features cellgov_cli/decrypt
}

# Each lane builds in a background subshell, where a failing command
# ends that lane alone; the gate waits for both, prints the release
# lane's output, and fails with the first failure before any test runs.
gate() {
    local release_log=$gate_root/gate-release.log debug_pid release_pid status=0 release_status=0
    timed fmt cargo fmt --check
    if [ "$gate_mode" = docs ]; then
        gate_debug_build
        return
    fi
    mkdir -p "$gate_root"
    gate_release_build >"$release_log" 2>&1 &
    release_pid=$!
    gate_debug_build &
    debug_pid=$!
    wait "$debug_pid" || status=$?
    wait "$release_pid" || release_status=$?
    [ "$status" -ne 0 ] || status=$release_status
    cat "$release_log"
    if [ "$status" -ne 0 ]; then
        group_line " (failed)" "$status"
        exit "$status"
    fi
    (gate_debug_tests)
    (gate_release_tests)
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

# The pre-push gate, local only: every group the workflow runs plus
# `local` on the pushed commit, the `check` group on the MSRV, and the
# Linux allocation gate in Docker. It runs on the commit's files alone,
# in one persistent detached worktree beside the repository: a forced
# checkout moves it, so only changed files get new mtimes, and a clean
# then removes every untracked and ignored file, so an operator input
# never satisfies a check. Its builds go to target/push and stay
# incremental from one push to the next. A tree that passes is recorded
# under target/push/passed with the toolchains it passed on; a later
# push of the same tree on the same toolchains runs only `deny`, whose
# advisory database moves without the tree.
push_gate_key() {
    printf '%s\n' "$1" "$(rustc --version)" "$(rustup run "$2" rustc --version)" \
        "$(cargo nextest --version | head -n 1)" \
        "$(docker image inspect "rust:$2" --format '{{.Id}}' 2>/dev/null || true)" |
        sha256sum | cut -c1-16
}

push_gate() {
    local commit=$1 repo wt td msrv tree record docker_toolchain
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
    repo=$(git rev-parse --show-toplevel)
    wt=${CELLGOV_PUSH_WORKTREE:-$repo-push}
    td=$repo/target/push
    git worktree prune
    if [ -f "$wt/.git" ]; then
        git -C "$wt" checkout -q --detach --force "$commit"
        git -C "$wt" clean -q -fdx
    elif [ -e "$wt" ]; then
        # Only a linked worktree has a .git file; the clean above must
        # never reach the main checkout or any other directory.
        echo "ci.sh: push-gate: $wt exists and is not a linked worktree" >&2
        return 1
    else
        git worktree add -q --detach "$wt" "$commit"
    fi
    cd "$wt"
    msrv=$(sed -n 's/^rust-version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
    test -n "$msrv"
    case "$msrv" in
        *.*.*) docker_toolchain=$msrv ;;
        *) docker_toolchain=$msrv.0 ;;
    esac
    docker info >/dev/null
    tree=$(git rev-parse "$commit^{tree}")
    record=$td/passed/$tree-$(push_gate_key "$tree" "$msrv")
    # No debuginfo, as the workflow builds: debug test builds link
    # several times faster without it.
    export CARGO_TARGET_DIR=$td CELLGOV_RUSTDOC_CHECK=1 CARGO_PROFILE_DEV_DEBUG=0
    if [ -f "$record" ]; then
        echo "ci.sh: push-gate: tree $tree passed $(cat "$record") on these toolchains; deny only"
        bash .github/ci.sh deny
        return
    fi
    bash .github/ci.sh full
    # The workflow's msrv job runs this group on the same toolchain. An
    # unavailable toolchain fails here rather than in the workflow.
    RUSTUP_TOOLCHAIN=$msrv bash .github/ci.sh check
    # Windows cannot show Linux-only allocator or filesystem behavior.
    echo "ci.sh: push-gate: Linux MSRV allocation gate"
    MSYS_NO_PATHCONV=1 docker run --rm \
        --mount "type=bind,source=$(pwd -W 2>/dev/null || pwd),target=/workspace" \
        --mount "type=volume,source=cellgov-cargo-registry,target=/usr/local/cargo/registry" \
        --workdir /workspace \
        --env CARGO_TARGET_DIR=/tmp/cellgov-target \
        --env "RUSTUP_TOOLCHAIN=$docker_toolchain" \
        "rust:$msrv" \
        cargo test -p cellgov_core --release --locked --test allocation_counts
    # Keyed after the run: the run pulls the image when it is missing.
    record=$td/passed/$tree-$(push_gate_key "$tree" "$msrv")
    mkdir -p "$td/passed"
    echo "at $(date -u +%Y-%m-%dT%H:%M:%SZ) as $commit" >"$record"
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
    push-gate) push_gate "${2:?usage: $0 push-gate <commit>}" ;;
    gate-tests | gate-docs | gate-full)
        gate_mode=${1#gate-}
        run_group "$1" gate
        ;;
    # The pre-push gate: every group the workflow runs, plus `local`. The
    # test group runs through nextest as the workflow's blocking legs do,
    # so a test over their per-test budget fails here too.
    full)
        run_group lint lint
        run_group deny deny
        CELLGOV_TEST_RUNNER=nextest run_group test test_suite
        run_group test-linux test_linux
        run_group local test_local
        ;;
    *)
        echo "usage: $0 {lint|check|test|test-linux|local|smoke-artifacts|deny|micro-build|microtests|data|full|gate-tests|gate-docs|gate-full|push-gate <commit>}" >&2
        exit 2
        ;;
esac
