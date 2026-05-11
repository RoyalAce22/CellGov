# RPCS3 CellGov Patches

Opt-in hooks the patched RPCS3 build exposes for cross-runner
investigation. There are currently two:

| Patch | Purpose |
| --- | --- |
| `0001-cellgov-checkpoint-dump.patch` | Memory dump at `_sys_process_exit` for observation comparison. |
| `0002-cellgov-hle-trace.patch` | Per-HLE-call trace stream with watch-address diff. Lets `cellgov_cli rpcs3-attribute` answer "which HLE call wrote this guest address?" in one run. |

Apply both:

```bash
cd tools/rpcs3-src
# 0001 is a self-contained unified diff.
git apply ../../bridges/rpcs3-patch/0001-cellgov-checkpoint-dump.patch
# 0002 has new files plus a unified diff against existing files.
cp ../../bridges/rpcs3-patch/files/Emu/Cell/cellgov_hle_trace.h   rpcs3/Emu/Cell/
cp ../../bridges/rpcs3-patch/files/Emu/Cell/cellgov_hle_trace.cpp rpcs3/Emu/Cell/
git apply ../../bridges/rpcs3-patch/0002-cellgov-hle-trace.patch
```

Then rebuild per RPCS3's normal instructions. The build artifact
goes at `tools/rpcs3-src/build-msvc/bin/rpcs3.exe` and must be
copied to `tools/rpcs3/rpcs3.exe`.

The new C++ files for `0002` live under
`bridges/rpcs3-patch/files/Emu/Cell/` so they survive in version
control; `tools/rpcs3-src/` is fully gitignored. Treat the
tracked `bridges/rpcs3-patch/files/` tree as the source of truth
when editing the trace hook.

# 0001: Checkpoint Dump Patch

## What it does

Adds a hook in `rpcs3/Emu/Cell/lv2/sys_process.cpp` (the
`_sys_process_exit` syscall) that writes configured guest-memory
regions to a binary file. The hook is a no-op unless both
`CELLGOV_DUMP_PATH` and `CELLGOV_DUMP_REGIONS` environment variables
are set. No behavior change when unset.

## Applying the patch

```bash
cd tools/rpcs3-src
git apply ../rpcs3-patch/0001-cellgov-checkpoint-dump.patch
```

Rebuild RPCS3 per its normal build instructions.

## Running with the hook active

RPCS3 must be configured in oracle mode before the dump is
comparable. The canonical settings are checked in at
`oracle_mode_config.yml`; `rpcs3-to-observation` refuses dumps
whose `--config-hash` does not match the hash of that file.

```bash
export CELLGOV_DUMP_PATH=/tmp/flow_rpcs3.dump
export CELLGOV_DUMP_REGIONS=0x10000:0x800000,0x10000000:0x400000
./rpcs3 path/to/EBOOT.elf
```

Each region is written contiguously in declaration order. The region
manifest passed to `rpcs3-to-observation` must list the same regions
in the same order -- the dump file has no internal structure beyond
that contract.

To convert the dump, ask the bridge for the expected config hash
and pass it alongside the dump:

```bash
EXPECTED=$(cargo run -q -p rpcs3_to_observation -- --print-expected-config-hash)
cargo run -q -p rpcs3_to_observation -- \
    --dump /tmp/flow_rpcs3.dump \
    --manifest tests/fixtures/NPUA80001_checkpoint.toml \
    --outcome completed \
    --output /tmp/flow_rpcs3.json \
    --config-hash "$EXPECTED"
```

## Design notes

Read-only with respect to guest state: the hook reads `vm::base(addr)`
and writes to a host file. No guest memory is modified.

Hook point is `_sys_process_exit`, chosen because both CellGov and
RPCS3 reach it deterministically at the same architectural boundary
during flOw's boot. A PC-based hook would be brittle across RPCS3
versions; a syscall-based one is stable.

Not upstream-quality as written. The final upstreamable form should:

- Use RPCS3's `fs::file` abstraction instead of raw `FILE*`.
- Route errors through `sys_log` instead of `stderr`.
- Move the env-var parse into a one-shot initializer at emulator
  start.

CellGov accepts the simpler form until we know whether per-step
emission requires a second patch; a single consolidated upstream
submission is preferable to two small ones.

# 0002: HLE Trace Patch

## What it does

Hooks the `BIND_FUNC` macro in `rpcs3/Emu/Cell/PPUFunction.h` to
emit a structured record per HLE module call. Records carry the
function name, r3..r10 args at entry, r3 return value, and -- if
a watch list is supplied via `CELLGOV_HLE_WATCH` -- the bytes
that changed at any watch address during the call (diff against
the entry-time snapshot).

No-op unless `CELLGOV_HLE_TRACE_PATH` is set. Cost on the
non-tracing path is one bool check per HLE call.

## Running with the hook active

```bash
export CELLGOV_HLE_TRACE_PATH=/tmp/flow.htrc
export CELLGOV_HLE_WATCH=0x101e3cb8:8
./rpcs3 --headless path/to/EBOOT.elf
```

Multiple watch addresses are comma-separated:
`CELLGOV_HLE_WATCH=0x101e3cb8:8,0x10400000:0x100`. Sizes are hex
with optional `0x` prefix; the per-region cap is 1 MiB.

## Consuming the trace

```bash
# "Which HLE call wrote 0x101e3cb8?"
cellgov_cli rpcs3-attribute --trace /tmp/flow.htrc --addr 0x101e3cb8

# Watch a multi-byte field; --len in hex.
cellgov_cli rpcs3-attribute --trace /tmp/flow.htrc --addr 0x101e3ca0 --len 0x20

# Rank HLE functions by total writes (which calls do real work?).
cellgov_cli rpcs3-attribute --trace /tmp/flow.htrc --ranked

# Dump every record (verbose; useful for narrow watch lists).
cellgov_cli rpcs3-attribute --trace /tmp/flow.htrc --list
```

The `--addr` query returns hits sorted by step ascending, so the
FIRST result is the earliest HLE call that touched the address;
later hits indicate overwrites by subsequent calls.

## Design notes

The `BIND_FUNC` macro wraps every HLE module function with thread
context and `current_function` name tracking, so the hook lives
naturally there. Nested HLE calls (HLE function A calling HLE
function B) emit a record at every level; the consumer can group
by step and depth to find the deepest-touching frame for precise
attribution.

The trace covers HLE module calls (cellGcm*, cellSysmodule*, etc.)
but NOT raw LV2 syscalls, which dispatch through a different path
(`lv2.cpp` `BIND_SYSC` macro). If a future investigation needs
syscall-level attribution, the same hook shape transplants there.

Read-only with respect to guest state: snapshots use `vm::base()`
+ memcpy to a host buffer, never write guest memory back. No
scheduler perturbation: hooks run on the calling PPU thread, do
no sleeps, and do not change `ppu.state`.

The trace file format is little-endian and self-describing
(magic + version). Format pinned in
`tools/rpcs3-src/rpcs3/Emu/Cell/cellgov_hle_trace.h` and parsed by
`apps/cellgov_cli/src/cli/rpcs3_attribute.rs`. Bumping the version
is a coordinated change across both files.

Not upstream-quality as written. Same gaps as 0001 (raw `FILE*`,
no `sys_log`, env-var parse on first use rather than at emulator
start). Acceptable for in-tree investigation; gets cleaned up
together with 0001 if either ever goes upstream.
