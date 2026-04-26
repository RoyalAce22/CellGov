# WipEout HD Fury Cross-Runner Comparison: Reproduction

How to reproduce a CellGov-vs-RPCS3 observation comparison on
WipEout HD Fury (BCES00664) at the FirstRsxWrite boot checkpoint.

## Why JSON observations are not committed

A CellGov observation JSON for WipEout's boot checkpoint is ~110 MB
(9.5 MB of raw bytes inflated by JSON's integer-array encoding).
The RPCS3-side dump is comparable. Both are produced locally; only
this README and `compare_report.txt` are checked in.

## Prerequisite: decrypted EBOOT.elf

WipEout ships as a disc ISO with an encrypted EBOOT.BIN. CellGov
loads a decrypted ELF, so the SELF must be decrypted once:

```bash
cargo run --release -p cellgov_firmware -- decrypt-self \
    tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.BIN
```

This writes `EBOOT.elf` (9.2 MB) next to `EBOOT.BIN`. The
manifest's `eboot_candidates` lists the .elf first, so once
decrypted CellGov's run-game picks it up automatically.

## CellGov side

```bash
cargo build --release -p cellgov_cli

./target/release/cellgov_cli run-game \
    --title wipeout \
    --max-steps 100000000 \
    --save-observation tests/fixtures/BCES00664_cross_runner/cellgov.json \
    --observation-manifest tests/fixtures/BCES00664_checkpoint.toml
```

Produces `cellgov.json` with 2 regions (code, data) matching the
manifest, ~110 MB, outcome RsxWriteCheckpoint at 20,569 steps
(~5.3M instructions, ~1s wall time).

## RPCS3 side (requires patched build)

See `tests/fixtures/NPUA80001_cross_runner/REPRODUCTION.md` for
build instructions. The same patch and binary works for WipEout.

The patched binary supports CELLGOV_DUMP_PATH, CELLGOV_DUMP_REGIONS,
and CELLGOV_DUMP_TTY_NTH. For WipEout we trigger on TTY write #20,
which lands past the asterisk-banner stage and before the game's
RSX-init stage:

```bash
cd tools/rpcs3
CELLGOV_DUMP_PATH=../../tests/fixtures/BCES00664_cross_runner/rpcs3.dump \
CELLGOV_DUMP_REGIONS=0x10000:0x848e48,0x860000:0xd6f80 \
CELLGOV_DUMP_TTY_NTH=20 \
./rpcs3.exe --headless \
    "D:/Emulation/ROMs/PS3/WipEout HD Fury (Europe) (En,Fr,De,Es,It,Nl,Pt,Sv,No,Da,Fi,Ru).iso"
```

The patched RPCS3 mounts the ISO via games.yml (which maps
BCES00664 to the ISO path) and boots through to the dump trigger.
On success the binary prints:

```
[cellgov] checkpoint dump written to ../../tests/fixtures/BCES00664_cross_runner/rpcs3.dump
[cellgov] dumped at sys_tty_write call #20; exiting RPCS3
```

The dump is exactly 9,567,688 bytes (sum of region sizes).

### Convert the dump to an Observation JSON

```bash
cargo build --release -p rpcs3_to_observation

./target/release/rpcs3_to_observation \
    --dump tests/fixtures/BCES00664_cross_runner/rpcs3.dump \
    --manifest tests/fixtures/BCES00664_checkpoint.toml \
    --outcome completed \
    --output tests/fixtures/BCES00664_cross_runner/rpcs3.json \
    --config-hash 0x78007ecdcdaaeb38
```

The config-hash matches `bridges/rpcs3-patch/oracle_mode_config.yml`;
print the expected value with
`./target/release/rpcs3_to_observation --print-expected-config-hash`.

## Compare

```bash
./target/release/cellgov_cli compare-observations \
    tests/fixtures/BCES00664_cross_runner/cellgov.json \
    tests/fixtures/BCES00664_cross_runner/rpcs3.json
```

Expected output (saved to `compare_report.txt`):

```
DIVERGE region code: first byte differs at offset 0x17 (guest 0x10017) -- 00 vs 01
DIVERGE region data: first byte differs at offset 0xffee (guest 0x86ffee) -- 00 vs 1c
```

See `compare_report.txt` for the full classification: code-side
divergences are ELF metadata reconstruction differences plus
unpopulated SYS_PROC_PARAM bytes; data-side is the HLE OPD pointer
table layout (same class as flOw and SSHD).

## Status

- **CellGov side**: reproducible with the command above. Verified
  locally: 2 regions / 9,567,688 bytes, outcome RsxWriteCheckpoint,
  20,569 steps.
- **RPCS3 side**: reproducible with the patched build above. Verified
  locally: 2 regions / 9,567,688 bytes.
- **Cross-runner compare**: 14 byte diffs in code (2 ELF metadata,
  12 PROC params), 960 byte diffs in data (HLE OPD layout). All
  divergences classified non-semantic.
