#!/usr/bin/env bash
# Self-hosted external-data gate. Command logs stay on the runner; Actions sees
# only statuses and the aggregate anchor tally.

set -uo pipefail

external_data_features="${EXTERNAL_DATA_FEATURES:-$(sed -n '/^[[:space:]]*EXTERNAL_DATA_FEATURES: >-/{n;p;}' .github/workflows/ci.yml | tr -d '[:space:]')}"
test -n "$external_data_features" || { echo "external-data tests: configuration failed"; exit 2; }
test -n "${CELLGOV_CI_LOG_DIR:-}" || { echo "external-data tests: runner log directory is not configured"; exit 2; }
test -n "${CELLGOV_PS3_VFS_ROOT:-}" || { echo "anchor regression check: VFS root is not configured"; exit 2; }

run_dir="$CELLGOV_CI_LOG_DIR/${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
mkdir -p "$run_dir"

tests_status=0
cargo test --workspace --locked --features "$external_data_features" \
  >"$run_dir/external-data-tests.log" 2>&1 || tests_status=$?
if [ "$tests_status" -eq 0 ]; then
  echo "external-data tests: pass"
else
  echo "external-data tests: fail (exit $tests_status)"
fi

build_status=0
cargo build --release --locked -p cellgov_cli --features decrypt \
  >"$run_dir/anchor-build.log" 2>&1 || build_status=$?

anchor_status=0
if [ "$build_status" -eq 0 ]; then
  target/release/cellgov.exe --vfs-root "$CELLGOV_PS3_VFS_ROOT" \
    --no-color --no-progress --no-input boot bench --all --runs 1 \
    >"$run_dir/anchor-sweep.log" 2>&1 || anchor_status=$?
  tally=$(grep -E '^boot bench --all:' "$run_dir/anchor-sweep.log" | tail -n 1)
  if [ -n "$tally" ]; then
    echo "$tally"
  else
    echo "boot bench --all: no tally (exit $anchor_status)"
  fi
else
  anchor_status=$build_status
  echo "boot bench --all: build failed (exit $build_status)"
fi

if [ "$tests_status" -ne 0 ] || [ "$anchor_status" -ne 0 ]; then
  exit 1
fi
