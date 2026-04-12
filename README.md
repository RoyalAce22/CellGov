# CellGov

A deterministic Rust runtime for translated PS3 Cell execution units.

[![CI](https://img.shields.io/github/actions/workflow/status/RoyalAce22/CellGov/ci.yml?branch=main&label=CI)](https://github.com/RoyalAce22/CellGov/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/RoyalAce22/CellGov/branch/main/graph/badge.svg)](https://codecov.io/gh/RoyalAce22/CellGov)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![Status: experimental](https://img.shields.io/badge/status-experimental-red.svg)](#status)

## Overview

CellGov is an experimental event-driven runtime for translated PS3 Cell workloads. Its focus is deterministic scheduling, shared-state visibility, and replayable execution for PPU and SPU-style execution units.

The project does not attempt to reproduce wall-clock timing. Instead, it models guest-visible causality: the order in which writes, DMA completions, mailbox messages, and synchronization events become observable across execution units.

At the center of the design is one rule:

> No translated execution unit may directly publish guest-visible shared state. All guest-visible effects must pass through the runtime commit pipeline.

## Scope

CellGov is **not** a full PS3 emulator. It does not currently aim to provide:

- RSX / graphics
- audio
- full LV2 support
- title-specific compatibility hacks

It is also not yet tied to any specific translation backend. The runtime contract comes first.

## Status

CellGov is in early development and currently in **Pre-Alpha**.

The current focus is:

- core runtime loop
- deterministic event ordering
- binary tracing and replay
- fake execution units for scenario testing

Interfaces are expected to change. The project is not yet suitable for real PS3 workloads.

## Workspace

CellGov is organized as a Cargo workspace with separate crates for:

- runtime orchestration
- time and event ordering
- memory and effect staging
- sync and DMA models
- execution-unit interfaces
- tracing and replay
- scenario-based testing

## Building

Requires Rust 1.85 or newer.

```bash
cargo build
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Testing

The test harness is scenario-driven and replay-oriented. The goal is to verify deterministic behavior through structured traces and final state hashes, rather than through human-readable logs.

## Documentation

Project documentation lives under [`docs/`](docs/). Start with:

- [`docs/architecture.md`](docs/architecture.md) -- the architecture overview, crate layering diagram, runtime pipeline, and determinism contract.

## License

Licensed under either of:

- Apache License, Version 2.0
- MIT license
