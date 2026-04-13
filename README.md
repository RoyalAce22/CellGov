# CellGov

Deterministic analysis engine for PS3 Cell Broadband Engine workloads.

[![CI](https://img.shields.io/github/actions/workflow/status/RoyalAce22/CellGov/ci.yml?branch=main&label=CI)](https://github.com/RoyalAce22/CellGov/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/RoyalAce22/CellGov/branch/main/graph/badge.svg)](https://codecov.io/gh/RoyalAce22/CellGov)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![Status: experimental](https://img.shields.io/badge/status-experimental-red.svg)](#status)

## What CellGov is

CellGov is a deterministic, event-driven Rust runtime that interprets PS3 PPU and SPU code, produces replayable execution traces, and validates its output against RPCS3 baselines. It is designed as the **foundation layer for static recompilation** of PS3 games to native binaries.

The deterministic interpreter and trace infrastructure live here. A separate downstream project will consume CellGov as a library and own the actual AoT/static recompiler (IR passes, register allocation, native codegen). CellGov provides the ground truth the recompiler targets and the comparison harness that validates recompiled output.

At the center of the design is one rule:

> No translated execution unit may directly publish guest-visible shared state. All guest-visible effects must pass through the runtime commit pipeline.

## What CellGov is not

CellGov is **not** a PS3 emulator and does not aim to run games at real-time speed. It does not provide:

- RSX / graphics, audio, networking, or input
- full LV2 kernel surface
- JIT or host-speed execution
- title-specific compatibility hacks

RPCS3 is the right tool if the goal is to play a game. CellGov is the right tool if the goal is to produce a byte-level-correct understanding of how a game executes -- which schedules are legal, which synchronization patterns are load-bearing, and what the correct output is under every legal interleaving.

## Why determinism matters for static recomp

PS3 games run PPU and SPU threads concurrently. A static recompiler must decide which synchronization to preserve and which is coincidental. CellGov answers that question:

- **Deterministic tracing** produces a complete, ordered record of every scheduling decision, effect, and commit. Two runs of the same scenario produce byte-identical traces.
- **Schedule exploration** systematically tries alternate legal interleavings and classifies which ones produce different outcomes. This tells the recompiler exactly which orderings the game depends on.
- **Oracle comparison** validates CellGov's output against RPCS3 baselines, and the same harness will validate recompiled output against CellGov's traces.

## Status

CellGov is in early development and currently in **Pre-Alpha**.

Current capabilities:

- deterministic round-robin scheduler with pluggable scheduler injection and deadlock detection
- commit pipeline processing 9 effect types (writes, mailbox, DMA, signals, wake/block, faults, trace markers) with fast-path skip for zero-effect steps
- real PPU interpreter with 79 instruction variants covering integer, FP, branch, compare, rotate/shift, VMX, load/store, and SPR/CR operations
- real SPU interpreter (128x128-bit register file, 256 KB local store, channel file)
- LV2 host model with 13 working syscalls (SPU lifecycle, mutex create/lock/unlock, event queue create/destroy, memory allocate/free, mailbox write, TTY write, process exit)
- firmware PRX loading: SPRX parser for decrypted PS3 firmware modules (ELF64 type 0xFFA4), segment loader with 4 relocation types (ADDR32, ADDR16_LO/HI/HA) using PS3 segment-relative encoding, export table extraction, and module_start execution through the PPU interpreter
- HLE import infrastructure: PRX import table parser, NID database (140+ functions across 12 PS3 modules with stub classification), 24-byte GOT-patching trampolines, NID-based runtime dispatch for TLS init, malloc, memset, and process exit, with HLE keep-list for functions that depend on incomplete firmware initialization
- kernel bootstrap: TLS pre-initialization from the game ELF's PT_TLS segment, bump-allocating kernel memory for sys_memory_allocate, monotonic ID allocation for kernel objects
- RuntimeMode enum (FaultDriven/DeterminismCheck/FullTrace) controlling trace and hash checkpoint overhead
- binary trace format with categorical filtering, encode/decode roundtrip, and mode-gated emission
- FNV-1a state hashing with cached content_hash and mode-gated checkpoints for large guest memories
- inline WritePayload storage (stack-allocated for payloads up to 16 bytes, heap fallback above)
- bounded schedule exploration with dependency-aware pruning and schedule-stable/sensitive classification
- oracle-aware exploration comparing per-schedule memory against RPCS3 baselines
- comparison harness with strict/memory/events/prefix modes and RPCS3 oracle validation
- six PSL1GHT-compiled microtests matching RPCS3 interpreter + LLVM baselines
- real game boot: flOw (NPUA80001) boots past C++ static initialization into game setup -- 337K+ PPU steps, 30K+ distinct PCs, 500+ HLE calls, with liblv2.prx loaded and module_start executed before the game entry point
- criterion benchmark harness for decode, execute, run_until_yield, content_hash, and commit_step with baseline comparison
- CLI with run, dump, compare, explore, and run-game subcommands (human + JSON output, per-step trace, instruction coverage, boot progress checkpoints, register dump and mini-trace on fault, HLE import classification summary, `--firmware-dir` for PRX loading)
- 857+ tests across 15 crates and two binaries, zero `unsafe`

## Workspace

CellGov is organized as a Cargo workspace. See [`docs/architecture.md`](docs/architecture.md) for the full crate layering diagram and per-crate responsibilities.

## Building

Requires Rust 1.85 or newer.

```bash
cargo build
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Testing

The test harness is scenario-driven and replay-oriented. Assertions run against structured trace records and final state hashes, not human-readable logs. The comparison harness validates CellGov observations against RPCS3 baselines using a normalized observation schema.

## Documentation

- [`docs/architecture.md`](docs/architecture.md) -- architecture overview, crate layering diagram, and runtime pipeline

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT license
