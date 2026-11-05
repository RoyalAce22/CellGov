#!/usr/bin/env bash
# Shared command groups for GitHub Actions and the local pre-push gate.
#
# Each command prints `ci.sh: <group>/<command> <seconds>s`, and each
# group prints `ci.sh: <group> <seconds>s`. Under GitHub Actions the same
# lines go to the step summary.
#
# The workflow and a local run differ only in:
# - CARGO_PROFILE_DEV_DEBUG=0 and CARGO_INCREMENTAL=0 in the workflow,
#   which change no check;
# - the toolchain each job installs;
# - the test runner of the blocking test legs (see test_suite);
# - `rustdoc --check` in the pre-push gate (see rustdoc_lint).

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
# A failure also prints the group line and exits with the command's
# status. CELLGOV_TIMING_RECORD=<file> appends one tab-separated row per
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
# Keep <function> out of `||` and `if`: there, bash disables `set -e` for
# every command inside it.
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
    rustdoc_lint doc
    rustdoc_lint doc-external-data --features "$external_data_features"
}

# rustdoc_lint <label> [cargo doc args...]: the workspace rustdoc lints.
# CELLGOV_RUSTDOC_CHECK=1 runs them through `rustdoc --check`: the same
# lint passes with no HTML. The HTML build costs most of a minute of one
# core per crate, to merge the search index. RUSTC_BOOTSTRAP=1 enables
# the unstable `--check` for this command only, in a target dir of its own.
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

# The test groups. Each distinct feature and package selection recompiles
# the crates whose features change, so each group runs where it costs least:
#
#   test        both platforms: the external-data check, the debug
#               workspace tests, and the release workspace tests with
#               `cellgov_cli/decrypt`
#   test-linux  the Linux stable leg: builds with no platform dependence
#   local       the pre-push gate only: the decrypt-off release tests and
#               the decrypt-on debug tests of the two crates that declare
#               the feature, so every pairing of profile and `decrypt`
#               runs before a push. Plain `cargo test` runs a binary's
#               tests as threads of one process, so a race over
#               process-global state stays visible.
#
# CELLGOV_TEST_RUNNER=nextest runs the workspace tests through the `ci`
# and `ci-release` profiles of .config/nextest.toml, with a per-test
# timeout. nextest runs no doctests, so `cargo test --doc` runs them.
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
    # The microtest post-link patcher tests. -B keeps __pycache__ out of
    # the tree.
    timed patch-toc python3 -B -m unittest discover -s tests/micro/common -p 'test_*.py'
}

test_local() {
    timed test-release cargo test --workspace --release --locked
    timed test-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked --features cellgov_cli/decrypt
}

# The operator's per-branch gate, local only: `gate-tests` is the
# compiled-output checkpoint, `gate-docs` the closing proof when only
# comments and prose changed since it, and `gate-full` both.
#
# - The gate and the operator's interactive builds use separate target
#   dirs, so neither invalidates the other.
# - The debug lane links with rust-lld and no debuginfo. After a
#   `cellgov_core` edit its test builds link in 12 s, against 136 s with
#   link.exe and full debuginfo. The linker variable applies on MSVC only.
# - The release lane builds as the workflow does, with incremental
#   compilation off. Incremental compilation changes which allocations
#   LLVM removes, and `allocation_counts` reads them.
# - The two lanes build side by side. The release tail is a few crates
#   at opt-level 3 that leave most cores idle.
# - No test runs before both builds end, and the suites run one at a
#   time. Tests beside the release build stalled 20 s at 35 GB of commit
#   on a 32 GB host, and hit the per-test timeout.
# - Each lane tests the decrypt side first, the side its build ends on.
#   A switch of `decrypt` in one target dir recompiles the `cellgov`
#   binary, about 20 s in release.
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
    if [ "$gate_mode" = full ]; then
        gate_rustdoc
    fi
    if [ "$gate_mode" != docs ]; then
        timed build-debug cargo nextest run --workspace --locked --profile ci --no-run
        timed build-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked \
            --features cellgov_cli/decrypt --no-run
    fi
}

gate_rustdoc() {
    gate_debug_env
    CELLGOV_RUSTDOC_CHECK=1 rustdoc_lint doc
    CELLGOV_RUSTDOC_CHECK=1 rustdoc_lint doc-decrypt --features cellgov_cli/decrypt
}

gate_release_build() {
    gate_release_env
    timed build-release cargo nextest run --workspace --release --locked --profile ci-release --no-run
    timed build-decrypt-release cargo test -p cellgov_cli -p cellgov_install --release --locked \
        --features cellgov_cli/decrypt --no-run
}

gate_debug_tests() {
    gate_debug_env
    timed test-decrypt-debug cargo test -p cellgov_cli -p cellgov_install --locked --features cellgov_cli/decrypt
    timed test-doc cargo test --workspace --locked --doc
    timed test-debug cargo nextest run --workspace --locked --profile ci
}

gate_release_tests() {
    gate_release_env
    timed test-decrypt-release cargo test -p cellgov_cli -p cellgov_install --release --locked \
        --features cellgov_cli/decrypt
    timed test-release cargo nextest run --workspace --release --locked --profile ci-release
}

# side_by_side <log> <first> <second>: runs both lanes at once and fails
# with the first failure. The second lane writes to <log>, which prints
# after both end.
side_by_side() {
    local log=$1 first_pid second_pid status=0 second_status=0
    mkdir -p "$gate_root"
    "$3" >"$log" 2>&1 &
    second_pid=$!
    "$2" &
    first_pid=$!
    wait "$first_pid" || status=$?
    wait "$second_pid" || second_status=$?
    [ "$status" -ne 0 ] || status=$second_status
    cat "$log"
    if [ "$status" -ne 0 ]; then
        group_line " (failed)" "$status"
        exit "$status"
    fi
}

gate() {
    timed fmt cargo fmt --check
    if [ "$gate_mode" = docs ]; then
        side_by_side "$gate_root/gate-rustdoc.log" gate_debug_build gate_rustdoc
        return
    fi
    side_by_side "$gate_root/gate-release.log" gate_debug_build gate_release_build
    (gate_debug_tests)
    (gate_release_tests)
}

# The smoke set's finding artifacts, where the workflow uploads them
# after a red test step. The set is deterministic, so this run finds
# what the failed test found. The directory starts empty: a stored
# artifact refuses a later finding with different evidence at its path.
smoke_artifacts() {
    rm -rf target/fuzz-smoke
    timed fuzz-smoke-debug cargo run -p cellgov_cli --locked -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/debug --regressions crates/cellgov_fuzz/regressions \
        --spu-references crates/cellgov_fuzz/tests/fixtures/spu_reference
    timed fuzz-smoke-release cargo run -p cellgov_cli --locked --release -- dev fuzz smoke \
        --artifacts-dir target/fuzz-smoke/release --regressions crates/cellgov_fuzz/regressions \
        --spu-references crates/cellgov_fuzz/tests/fixtures/spu_reference
}

# The MSRV build check. The workflow's msrv job and the pre-push gate run
# this group on the MSRV toolchain. Tests run on the pinned and stable
# toolchains.
check() {
    timed check cargo check --workspace --all-targets --locked
    timed check-external-data cargo check --workspace --all-targets --locked --features "$external_data_features"
}

deny() {
    timed deny cargo deny check advisories bans licenses sources
}

# Runs each tests/micro/<name>/build.sh in <image>. The ELFs go to its
# gitignored build/ directory, which the microtest suites read. The
# workflow's microtests job runs `micro-build` when its cache of those
# directories misses. Each log goes to target/micro-build/<name>.log and
# prints only on failure.
build_every_microtest() {
    local image=$1 root dir name built=0
    # Docker on Windows needs a drive path. MSYS_NO_PATHCONV stops Git
    # Bash from rewriting the container paths.
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

# The targets that hold a microtest-gated test, with the features on.
# These suites need the operator's vfs/ and run only under
# `installed-firmware-tests`, which this group leaves off:
# - the spawn suite;
# - the SPURS suite;
# - the boot cases that load a firmware or read the key vault.
microtests() {
    timed test-microtests cargo nextest run --locked --profile ci-microtests \
        -p cellgov_ppu -p cellgov_spu -p cellgov_cli \
        --features cellgov_ppu/ppu-microtests,cellgov_spu/spu-microtests,cellgov_cli/microtests \
        -E 'package(cellgov_ppu) & kind(lib) | package(cellgov_spu) & (kind(lib) | test(/^spu_float_edges::/)) | binary_id(cellgov_cli::microtests) | binary_id(cellgov_cli::bin/cellgov) & test(=cli::scenarios::tests::explore_micro_runs_for_elf_microtests)'
}

# The data-feature suites, on the operator's machine: every target and
# test behind an external-data feature, through nextest's `ci-data`
# profile. The live-console suite (`runner_ps3/ps3-hardware`) stays out:
# it drives the PS3, so it runs only on the operator's request.
#
# Each run, pass or fail, appends one line to vfs/.cellgov/data-runs.tsv
# (CELLGOV_DATA_LEDGER moves it): the UTC time, the commit, the
# installed firmware and titles, and nextest's summary.
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

# The image the pre-push gate runs the microtests group in: the pinned
# toolchain, with the cargo-nextest version of the workflow's microtests
# job. It prints the image's tag and the two versions.
microtests_image() {
    local toolchain nextest
    toolchain=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
    # Each workflow job that installs cargo-nextest pins the same version.
    nextest=$(sed -n 's/^ *tool: cargo-nextest@//p' .github/workflows/ci.yml | sort -u)
    # A command substitution does not inherit `set -e`, so the check returns.
    if [ -z "$toolchain" ] || [ -z "$nextest" ] || [ "$(printf '%s\n' "$nextest" | wc -l)" -ne 1 ]; then
        echo "ci.sh: no pinned toolchain, or not one cargo-nextest version, in the workflow" >&2
        return 1
    fi
    echo "cellgov-microtests:$toolchain-nextest-$nextest $toolchain $nextest"
}

# The workflow's microtests job, for the pre-push gate, on the commit's
# files in the current directory.
#
# - `git clean -fdx` deletes the built ELFs. A tar of them in
#   $CARGO_TARGET_DIR/micro-build, named by the `tests/micro` tree,
#   restores them, as the workflow's cache keyed by `tests/micro/**`
#   does. A miss runs `micro-build` and saves the tar.
# - The suites run on Linux, as in the workflow. A Windows boot runs about
#   2.5 times slower than one on the runner, and the boot-suite walks
#   then pass their budget.
# - The container gets the runner's 4 CPUs. At most 4 boots run at once,
#   as the runner's 4 CPUs allow. Docker's VM can hold less memory than
#   the runner, and a Linux boot touches little of what it commits.
push_microtests() {
    local cache spec image toolchain nextest
    cache=$CARGO_TARGET_DIR/micro-build/$(git rev-parse HEAD:tests/micro).tar
    if [ -f "$cache" ]; then
        timed restore-elfs tar --force-local -xf "$cache"
    else
        # A tag of its own leaves the operator's `ps3dev` image alone.
        MICRO_IMAGE=cellgov-push-micro-toolchain bash .github/ci.sh micro-build
        mkdir -p "$CARGO_TARGET_DIR/micro-build"
        rm -f "$CARGO_TARGET_DIR"/micro-build/*.tar
        timed save-elfs tar --force-local -cf "$cache" tests/micro/*/build
    fi
    spec=$(microtests_image)
    read -r image toolchain nextest <<<"$spec"
    printf 'FROM rust:%s\nRUN curl -LsSf https://get.nexte.st/%s/linux | tar zxf - -C /usr/local/cargo/bin\n' \
        "$toolchain" "$nextest" >"$CARGO_TARGET_DIR/microtests.Dockerfile"
    # A Dockerfile on stdin builds with no context.
    timed image docker build -q -t "$image" - <"$CARGO_TARGET_DIR/microtests.Dockerfile"
    timed microtests env MSYS_NO_PATHCONV=1 docker run --rm --cpus 4 \
        --mount "type=bind,source=$(pwd -W 2>/dev/null || pwd),target=/workspace" \
        --mount "type=volume,source=cellgov-cargo-registry,target=/usr/local/cargo/registry" \
        --mount "type=volume,source=cellgov-push-microtests-target,target=/target" \
        --workdir /workspace \
        --env CARGO_TARGET_DIR=/target \
        --env "RUSTUP_TOOLCHAIN=$toolchain" \
        --env CARGO_PROFILE_DEV_DEBUG=0 \
        --env CELLGOV_MICROTESTS_MAX_CONCURRENT=4 \
        "$image" bash .github/ci.sh microtests
}

# The pre-push gate, local only, on the pushed commit: every workflow
# group plus `local`, `check` on the MSRV, and the Linux allocation gate
# in Docker. The microtests job's groups run on Linux (see
# push_microtests).
#
# - It runs in one persistent detached worktree beside the repository.
#   A forced checkout gives new mtimes to changed files only, so the
#   builds in target/push stay incremental.
# - `git clean -fdx` removes every untracked and ignored file, so no
#   operator input can satisfy a check.
# - target/push/passed records each tree that passes, with its
#   toolchains. A later push of that tree on those toolchains runs only
#   `deny`, whose advisory database changes without the tree.
push_gate_key() {
    printf '%s\n' "$1" "$(rustc --version)" "$(rustup run "$2" rustc --version)" \
        "$(cargo nextest --version | head -n 1)" \
        "$(docker image inspect "rust:$2" --format '{{.Id}}' 2>/dev/null || true)" \
        "$(docker image inspect "$(microtests_image | cut -d' ' -f1)" --format '{{.Id}}' 2>/dev/null || true)" |
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
        # Only a linked worktree has a .git file. This check keeps the
        # clean above away from the main checkout and every other directory.
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
    # No debuginfo, as in the workflow: debug test builds link several
    # times faster.
    export CARGO_TARGET_DIR=$td CELLGOV_RUSTDOC_CHECK=1 CARGO_PROFILE_DEV_DEBUG=0
    if [ -f "$record" ]; then
        echo "ci.sh: push-gate: tree $tree passed $(cat "$record") on these toolchains; deny only"
        bash .github/ci.sh deny
        return
    fi
    bash .github/ci.sh full
    run_group microtests-linux push_microtests
    # The workflow's msrv job runs this group on the same toolchain, so a
    # missing toolchain fails here first.
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
    # The key includes the image ids, and the runs build or pull a missing
    # image, so the key comes after the runs.
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
    # The workflow's groups but the microtests job's, plus `local`. The
    # pre-push gate adds those two on Linux (push_microtests). The test
    # group runs through nextest, as the workflow's blocking legs do, so
    # a test over the per-test budget fails here too.
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
