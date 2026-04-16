# RPCS3 Checkpoint Dump Patch

Opt-in memory dump hook for cross-runner observation comparison.
Produces the RPCS3-side input to `bridges/rpcs3_to_observation/`.

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
