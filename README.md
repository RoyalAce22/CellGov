# CellGov

Deterministic analysis engine for PS3 Cell Broadband Engine workloads.

[![CI](https://img.shields.io/github/actions/workflow/status/RoyalAce22/CellGov/ci.yml?branch=main&label=CI)](https://github.com/RoyalAce22/CellGov/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.95-orange.svg)](https://blog.rust-lang.org/2026/04/03/Rust-1.95.0.html)
[![Status: pre-alpha](https://img.shields.io/badge/status-pre--alpha-orange.svg)](#status)

## What CellGov is

CellGov interprets PS3 PPU and SPU code deterministically, produces
replayable execution traces, and validates its output against RPCS3
baselines. It is the **foundation layer for static recompilation** of
PS3 games to native binaries: not the recompiler itself, but the oracle
that tells the recompiler what the correct output is and which
synchronization patterns it must preserve.

The design rule at the center:

> Determinism comes from one rule: nothing a thread does is "live"
> until the runtime says so. Threads propose changes; the runtime is
> the only thing that ever applies them.

## What CellGov is not

CellGov does not run games. There is no RSX, no audio, no networking,
no input, no JIT, no host-speed execution, and no per-title
compatibility hacks. RPCS3 is the right tool to play a game. CellGov
is the right tool to ask, byte-for-byte, what a PS3 game would produce
under any legal schedule.

## Why determinism matters for static recomp

PS3 games run PPU and SPU threads concurrently. A static recompiler
must decide which synchronization to preserve and which is incidental.
CellGov answers that question:

- **Deterministic tracing.** Two runs of the same scenario produce
  byte-identical traces of every scheduling decision, effect, and
  commit.
- **Schedule exploration.** Bounded enumeration of legal interleavings,
  classified by whether they produce different observable outcomes.
- **Oracle comparison.** A normalized observation schema lets CellGov
  cross-validate against RPCS3, and lets a downstream recompiler
  cross-validate its output against CellGov.

## Status

Pre-Alpha. Capability today:

- **Titles: 3** -- flOw (NPUA80001), Super Stardust HD (NPUA80068),
  WipEout HD Fury (BCES00664). All three boot to their checkpoints
  with cross-runner observation match against RPCS3 (modulo
  classified non-semantic divergences). See [docs/titles.md](docs/titles.md)
  for the per-title compatibility matrix. Manifest-driven: adding a
  title is one TOML file, no Rust change.
- **PPU: 119 instruction variants**, including 5 quickened forms
  (Li, Mr, Slwi, Srwi, Clrlwi), 3 doubleword quickenings (Clrldi,
  Sldi, Srdi), and 9 superpairs (LwzCmpwi, LiStw, MflrStw, LwzMtlr,
  CmpwiBc, CmpwBc, MflrStd, LdMtlr, StdStd). Full SPU interpreter.
- **LV2: 22 syscalls**, 20 HLE exports with dedicated handling
  (including cellGcmSys RSX init cluster, memory-info queries,
  and the PPU thread lifecycle: create / exit / join / yield /
  get-id, with a per-thread table, stack allocator, and TLS
  template).
- **Multi-PPU threading.** `sys_ppu_thread_create` spawns a
  child `PpuExecutionUnit` mid-run; the deterministic
  round-robin scheduler interleaves all runnable PPU threads;
  `sys_ppu_thread_join` blocks the caller until the target
  exits and returns the exit value. Two-thread PSL1GHT
  microtest (`tests/micro/ppu_two_threads_disjoint_writes`)
  runs end-to-end with same-budget replay determinism and
  cross-budget final-state equivalence.
- **Memory**: PS3-spec sparse address space with store-forwarding
  buffer for intra-block coherence.
- **Throughput**: basic-block batching with mode-driven step budget
  (default 256, `--budget N` overrides) over a predecoded
  instruction shadow with quickening and super-pairing. The three
  wired titles boot at ~36-56M insns/sec to their checkpoints.
- **Tracing**: per-instruction state hashes, streaming divergence
  scanner, register-level zoom-in.
- **Cross-runner**: observation schema validated against RPCS3;
  reproducible boot bench with subprocess isolation; disc ISOs
  supported via `cellgov_firmware decrypt-self`.
- **Firmware**: standalone PUP decrypter (`cellgov_firmware`)
  extracts PS3 system modules from Sony's official firmware update
  without requiring RPCS3; also decrypts retail game SELFs to ELFs
  for disc-format titles.
- **NID database**: 5,327 PS3 library function name mappings
  merged from RPCS3's module registrations (99%+ of imports named
  in the per-title HLE inventories).
- 1,284 tests, zero `unsafe` (`unsafe_code = forbid`).

See [`docs/architecture.md`](docs/architecture.md) for full
technical details on the pipeline, memory model, shadow passes,
and effect vocabulary.

## Workspace

Cargo workspace, 15 library crates and 3 binaries (+1 firmware tool). See
[`docs/architecture.md`](docs/architecture.md) for the layering
diagram and per-crate responsibilities.

## Building

Requires Rust 1.95 or newer.

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

CellGov has no runtime dependency on RPCS3. Booting a real PS3 game
requires PS3 system firmware (decrypted SPRX modules like
`liblv2.sprx`). Download the official firmware update
(`PS3UPDAT.PUP`) from
[playstation.com](https://www.playstation.com/en-us/support/hardware/ps3/system-software/)
and decrypt it with the included tool:

```bash
cargo run -p cellgov_firmware -- install PS3UPDAT.PUP --output dev_flash
```

Then pass `--firmware-dir dev_flash/sys/external` to `run-game`.

The `cellgov_compare` crate gates the RPCS3 process-spawning runner
behind the default-on `rpcs3-runner` Cargo feature. Importers that
just want the `Observation` schema, `compare()`, `diverge()`, and
`zoom_lookup()` can opt out with
`default-features = false` and never compile RPCS3-aware code.

## Testing

Test assertions run against structured trace records and final state
hashes, never against human-readable logs. The comparison harness
validates CellGov observations against RPCS3 baselines through a
runner-agnostic observation schema.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT license
