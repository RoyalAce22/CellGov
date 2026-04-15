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

- Boots flOw (NPUA80001) past C++ static initialization, through
  liblv2's `module_start`, and reaches the first RSX call
  (`_cellGcmInitBody`) at PPU step 1.4M -- the documented CPU-side
  boundary for the static-recomp oracle.
- Cross-runner verified: at the first-`sys_tty_write` boot
  checkpoint, CellGov and RPCS3 produce byte-identical `code` and
  `rodata` segments. Remaining `data` divergences are bounded to
  HLE-binding-table slot ordering (a per-runner convention, not a
  semantic difference) and classified as ignored fields under the
  cross-runner divergence policy.
- PS3-spec guest memory layout: sparse `BTreeMap<u64, Region>`
  matching RPCS3 `vm.cpp`'s VA blocks. `sys_memory_allocate` returns
  pointers in `0x00010000-0x0FFFFFFF` (above the loaded ELF), the
  primary thread stack lives at `0xD0000000+`, and RSX/SPU-reserved
  ranges are addressable as zero-readable provisional regions.
- Per-step divergence trace: opt-in `PpuStateHash` records (one per
  retired PPU instruction), a streaming `diverge` scanner, and a
  zoom-in mode that names the exact register field that
  disagrees.
- Configurable HLE OPD packing: 24-byte per-binding trampolines for
  microtests, or 8-byte packed OPDs in user memory matching RPCS3's
  HLE-table shape for cross-runner comparison.
- 91 PPU instruction variants, full SPU interpreter, 15 LV2
  syscalls with non-default handling, NID-correct sysPrxForUser
  HLE dispatch.
- 978 tests across 15 library crates, 2 binaries, and 1 RPCS3
  bridge, zero `unsafe`.

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
