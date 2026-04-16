# CellGov

Deterministic analysis engine for PS3 Cell Broadband Engine workloads.

[![CI](https://img.shields.io/github/actions/workflow/status/RoyalAce22/CellGov/ci.yml?branch=main&label=CI)](https://github.com/RoyalAce22/CellGov/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![Status: experimental](https://img.shields.io/badge/status-experimental-red.svg)](#status)

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

- **Titles: 2** -- flOw (NPUA80001) cross-runner verified against
  RPCS3; Super Stardust HD (NPUA80068) boots past early init.
  Manifest-driven: adding a title is one TOML file, no Rust change.
- **PPU: 110 instruction variants**, including quickened
  specializations and superinstruction compounds. Full SPU
  interpreter.
- **LV2: 16 syscalls**, 10 HLE exports with dedicated handling.
- **Memory**: PS3-spec sparse address space with store-forwarding
  buffer for intra-block coherence.
- **Throughput**: basic-block batching (Budget=256), predecoded
  instruction shadow with quickening and super-pairing.
- **Tracing**: per-instruction state hashes, streaming divergence
  scanner, register-level zoom-in.
- **Cross-runner**: observation schema validated against RPCS3;
  reproducible boot bench with subprocess isolation.
- 1125 tests, zero `unsafe` (`unsafe_code = forbid`).

See [`docs/architecture.md`](docs/architecture.md) for full
technical details on the pipeline, memory model, shadow passes,
and effect vocabulary.

## Workspace

Cargo workspace, 15 library crates and 3 binaries. See
[`docs/architecture.md`](docs/architecture.md) for the layering
diagram and per-crate responsibilities.

## Building

Requires Rust 1.85 or newer.

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

The CellGov library crates have no external runtime dependencies on
RPCS3. Booting a real PS3 game requires PS3 system firmware files
(decrypted SPRX modules like `liblv2.sprx`); these are not shipped
with CellGov and must be supplied via `--firmware-dir`. RPCS3's
`dev_flash/sys/external` is one convenient source for the files,
but the dependency is on the PS3 firmware itself.

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
