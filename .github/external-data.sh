#!/usr/bin/env bash
# Self-hosted external-data gate. Command logs stay on the runner; Actions sees
# only statuses and the aggregate anchor tally.

set -uo pipefail

external_data_features="${EXTERNAL_DATA_FEATURES:-$(sed -n '/^[[:space:]]*EXTERNAL_DATA_FEATURES: >-/{n;p;}' .github/workflows/ci.yml | tr -d '[:space:]')}"
test -n "$external_data_features" || { echo "external-data tests: configuration failed"; exit 2; }
test -n "${CELLGOV_CI_LOG_DIR:-}" || { echo "external-data tests: runner log directory is not configured"; exit 2; }
test -n "${CELLGOV_PS3_VFS_ROOT:-}" || { echo "anchor regression check: VFS root is not configured"; exit 2; }
test -n "${CELLGOV_DUMPS_DIR:-}" || { echo "external-data tests: dump root is not configured"; exit 2; }

dumps_root="$(cygpath -u "$CELLGOV_DUMPS_DIR")"
missing_kernels=0
while IFS=$'\t' read -r pup_sha256 _; do
  [ "$pup_sha256" = "pup_sha256" ] && continue
  if [ ! -f "$dumps_root/lv2-census/$pup_sha256/lv2_kernel.elf" ]; then
    missing_kernels=$((missing_kernels + 1))
  fi
done < docs/lv2/tables/kernel.tsv
if [ "$missing_kernels" -ne 0 ]; then
  echo "external-data setup: $missing_kernels required LV2 kernel fixtures are missing"
  exit 2
fi

run_dir="$CELLGOV_CI_LOG_DIR/${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
mkdir -p "$run_dir"

# External-data tests use the checkout-relative default store so they remain
# fresh-clone safe when their features are off. Mount the operator store only
# for this feature-gated run, then remove the junction before the job exits.
workspace_root="$(cygpath -u "${GITHUB_WORKSPACE:-$PWD}")"
ps3_vfs_root="$(cygpath -u "$CELLGOV_PS3_VFS_ROOT")"
operator_vfs_root="$(dirname "$ps3_vfs_root")"
workspace_vfs="$workspace_root/vfs"
export CELLGOV_CI_WORKSPACE_VFS="$(cygpath -w "$workspace_vfs")"
export CELLGOV_CI_OPERATOR_VFS="$(cygpath -w "$operator_vfs_root")"

test "$(basename "$ps3_vfs_root")" = "dev_hdd0" || {
  echo "external-data tests: PS3 VFS root must name dev_hdd0"; exit 2;
}
test -d "$operator_vfs_root/.cellgov/installs" || {
  echo "external-data tests: operator store has no install records"; exit 2;
}
test ! -e "$workspace_vfs" || {
  echo "external-data tests: checkout VFS path already exists"; exit 2;
}

powershell.exe -NoLogo -NoProfile -NonInteractive -Command \
  '$ErrorActionPreference = "Stop"; New-Item -ItemType Junction -Path $env:CELLGOV_CI_WORKSPACE_VFS -Target $env:CELLGOV_CI_OPERATOR_VFS | Out-Null' || {
    echo "external-data tests: operator VFS mount failed"; exit 2;
  }

cleanup_operator_vfs() {
  fsutil.exe reparsepoint delete "$CELLGOV_CI_WORKSPACE_VFS" >/dev/null 2>&1 || return
  rmdir "$workspace_vfs"
}
trap cleanup_operator_vfs EXIT

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
