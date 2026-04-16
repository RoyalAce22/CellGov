# SSHD Cross-Runner Comparison: Reproduction

How to reproduce a CellGov-vs-RPCS3 observation comparison on
Super Stardust HD at the FirstRsxWrite boot checkpoint.

## Why JSON observations are not committed

A CellGov observation JSON for SSHD's boot checkpoint is ~98 MB
(8.4 MB of raw bytes inflated by JSON's integer-array encoding).
The RPCS3-side dump is comparable. Both are produced locally; only
this README and `compare_report.txt` are checked in.

## CellGov side

```bash
cargo build --release -p cellgov_cli

./target/release/cellgov_cli run-game \
    --title sshd \
    --max-steps 10000000000 \
    --save-observation tests/fixtures/NPUA80068_cross_runner/cellgov.json \
    --observation-manifest tests/fixtures/NPUA80068_checkpoint.toml
```

Produces `cellgov.json` with 2 regions (code, data) matching the
manifest, ~98 MB, outcome RsxWriteCheckpoint at 14,109,359 steps
(~3.6B instructions, ~72s wall time).

## RPCS3 side (requires patched build)

See `tests/fixtures/NPUA80001_cross_runner/REPRODUCTION.md` for
build instructions. The same patch and build works for SSHD.

The RPCS3 dump hook must be configured for SSHD's checkpoint
boundary. SSHD's shared boundary is the first write to the RSX
control register's put pointer (at 0xC0000040 in RSX space).

The dump-hook patch triggers on `sys_tty_write` by default. For
SSHD, the trigger may need adjustment: SSHD's tty output
("Eng", "UDP LOG:", "SPURS") occurs before the RSX write, so
`CELLGOV_DUMP_TTY_NTH` must be tuned or the hook must be
extended to trigger on RSX control register writes.

```bash
CELLGOV_DUMP_PATH=tests/fixtures/NPUA80068_cross_runner/rpcs3.dump \
CELLGOV_DUMP_REGIONS=0x10000:0x7c4f68,0x7e0000:0x49c98 \
./rpcs3.exe --no-gui --headless \
    dev_hdd0/game/NPUA80068/USRDIR/EBOOT.elf
```

### Convert the dump to an Observation JSON

```bash
cargo build --release -p rpcs3_to_observation

./target/release/rpcs3_to_observation \
    --dump tests/fixtures/NPUA80068_cross_runner/rpcs3.dump \
    --manifest tests/fixtures/NPUA80068_checkpoint.toml \
    --outcome completed \
    --output tests/fixtures/NPUA80068_cross_runner/rpcs3.json
```

## Compare

```bash
./target/release/cellgov_cli compare-observations \
    tests/fixtures/NPUA80068_cross_runner/cellgov.json \
    tests/fixtures/NPUA80068_cross_runner/rpcs3.json
```

## Status

- **CellGov side**: reproducible with the command above. Verified
  locally: 2 regions / 8,451,816 bytes, outcome RsxWriteCheckpoint,
  14,109,359 steps (~3.6B instructions).
- **RPCS3 side**: pending. Requires patched RPCS3 build with a
  dump trigger at the RSX control-register write boundary.
- **Cross-runner compare**: blocked on RPCS3 baseline.
