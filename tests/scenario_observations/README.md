# Scenario observations

RPCS3's recorded answers for the synthetic SPU scenarios.

Each file is an **observation** as
[`docs/concepts/`](../../docs/concepts/README.md) defines it: a typed JSON
snapshot of guest-visible state at a checkpoint.

Boot anchors are a separate tree. An anchor holds CellGov's own
witnesses for a real title at
`tests/fixtures/<content-id>/cellgov/boot_summary.json`. `boot bench`
gates against anchors. `dev record-anchors` writes them.

## Layout

```
tests/scenario_observations/<scenario>/rpcs3_interpreter.json
tests/scenario_observations/<scenario>/rpcs3_llvm.json
```

Six scenarios: `atomic_reservation`, `barrier_wakeup`,
`dma_completion`, `ls_to_shared`, `mailbox_roundtrip`,
`spu_fixed_value`.

Each name matches a directory under `tests/micro/`. That directory's
`manifest.toml` lists the `[observe] memory_regions` a dump contains.

## What reads them

| Consumer                                                       | Scenarios         |
| -------------------------------------------------------------- | ----------------- |
| `crates/cellgov_spu/src/tests/spu_tests.rs`                    | all six           |
| `crates/cellgov_ppu/src/tests/ppu_tests.rs`                    | by microtest name |
| `crates/cellgov_compare/src/tests/baseline_tests.rs`           | `spu_fixed_value` |
| `cellgov diff compare <manifest.toml> --observations-dir <dir>` | any               |
| `cellgov explore micro <name> --observations-dir <dir>`    | any               |

The SPU and PPU suites also need the built micro-test ELFs. They sit
behind `cellgov_spu/spu-microtests` and `cellgov_ppu/ppu-microtests`.
The JSON here is committed and needs no feature.

## Two decoders per scenario

Every scenario is recorded twice, once per RPCS3 decoder.
`compare_multi` refuses the comparison when the two disagree. A
CellGov-vs-RPCS3 divergence is therefore distinguishable from an
RPCS3-internal one. See `compare/driver.rs`.

`metadata.runner` records which decoder ran: `rpcs3-interpreter` or
`rpcs3-llvm`.

## `.tty` files are intermediates

A capture run writes a raw `.tty` dump on the way to the JSON.
`.gitignore` drops `tests/scenario_observations/**/*.tty`, so no `.tty`
survives a clone.

No test may read one. A test whose input is gitignored can only skip,
and a skip is indistinguishable from a pass.
`rpcs3_tty_baseline_roundtrip` in `baseline_tests.rs` shows the
supported shape: write the framed log into a scratch dir, then parse it
back.

## Regenerating one

Three steps. The whole set takes about twenty minutes, most of it
waiting on RPCS3.

**1. Configure RPCS3.** Apply every setting in the hashed block of
[`bridges/rpcs3_to_observation/oracle_mode_config.yml`](../../bridges/rpcs3_to_observation/oracle_mode_config.yml)
to `tools/rpcs3/config/config.yml`, then one decoder pair from the
`Decoders` map below it. Step 3 rejects a capture made under other
settings.

**2. Run the scenario.** The guest writes its result to TTY as a `CGOV`
frame, and RPCS3 logs that to `tools/rpcs3/log/TTY.log`.

```bash
rm -f tools/rpcs3/RPCS3.buf tools/rpcs3/log/TTY.log
MSYS_NO_PATHCONV=1 timeout 90 ./tools/rpcs3/rpcs3.exe --headless \
  tests/micro/<scenario>/build/<scenario>.elf
rm -f tools/rpcs3/RPCS3.buf
cp tools/rpcs3/log/TTY.log /tmp/<scenario>.tty
```

Two rules the surrounding `rm` lines exist to enforce. No second RPCS3
instance may be running, and no `RPCS3.buf` may be left over from a
previous run -- either one makes the capture unusable, and a stale buf
silently reuses the earlier run's state. `--headless` is the only
supported flag.

RPCS3 keeps running after the guest exits, which is why the `timeout`
and the two `rm` lines are there. The TTY frame lands within seconds;
the timeout only bounds the wait. Killing RPCS3 leaves `RPCS3.buf`
behind, and the wrapper refuses the next launch while it exists.

**3. Convert.** The bridge slices the frame into regions and writes the
observation.

```bash
cargo run -p rpcs3_to_observation --   --tty /tmp/<scenario>.tty   --manifest <regions.toml>   --outcome completed   --decoder llvm   --config-hash "$(cargo run -q -p rpcs3_to_observation -- --print-expected-config-hash)"   --output tests/scenario_observations/<scenario>/rpcs3_llvm.json
```

`<regions.toml>` uses the bridge's own format: one `[[regions]]` entry
per region with `name`, `addr`, and `size`, the last two as hex
strings, plus an optional `space` that must be 0 (RPCS3 runs one
guest process; the bridge refuses any other value). Copy them from the scenario's
`tests/micro/<scenario>/manifest.toml` `[observe] memory_regions`.
`addr` is the region's offset inside the emitted struct and the address
the observation reports, so regions can sit apart where the struct has
padding. `--outcome` is one of `completed|stalled|timeout|fault`.

Repeat with `--decoder interpreter` and the matching output filename to
produce the sibling. The two must agree: the bridge rejects a
`--decoder` that disagrees with the `_<decoder>.json` suffix.

The checkpoint dump hook plays no part here. `--dump` exists for
captures taken through it, which is how a real title's fixture is
recorded; these six scenarios report through TTY instead.

### Why the decoder sits outside the config hash

The hash covers the settings every capture shares, so a capture made
under the wrong renderer is rejected. The decoder is the axis this tree
varies. Hashing it would leave one of the two variants unreproducible.

## When to re-record

Re-record after the scenario itself changes: an edit to the micro-test
source, or a change to its `[observe] memory_regions` list. The old
observation then describes a program that no longer exists.

A failing comparison is a different case. The diff is the finding these
files exist to produce. Overwriting the reference deletes the evidence
and makes the suite agree with whatever CellGov does today. Localize
first with `cellgov diff diverge` on the state captures and
`cellgov diff zoom` on the step it names. Re-record only after you can
name the cause.

If the two decoders disagree with each other, re-recording either one
settles nothing. That is an RPCS3-internal divergence and needs its own
investigation.
