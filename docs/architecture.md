# CellGov Architecture

CellGov is a Rust workspace implementing a deterministic event-driven runtime for translated PS3 PPU and SPU execution units.

## Current state

The runtime executes units in a deterministic round-robin loop, processes effects through the commit pipeline, and produces a binary trace. An external-oracle comparison harness compares CellGov observations against RPCS3 baselines. The workspace compiles clean under `unsafe_code = "forbid"` and has 536 tests across 11 crates, a comparison crate, and a CLI.

### Runtime

- **Deterministic step loop** with round-robin scheduling and deadlock detection
- **Commit pipeline** processing 9 effect types: `SharedWriteIntent`, `MailboxSend`, `MailboxReceiveAttempt`, `DmaEnqueue`, `WaitOnEvent`, `WakeUnit`, `SignalUpdate`, `FaultRaised`, `TraceMarker`
- **Binary trace format** with 7 record types, categorical filtering, and encode/decode roundtrip
- **FNV-1a state hashing** at every commit boundary (committed memory, runnable queue, unit status, sync state)
- **DMA completion queue** with pluggable latency models and automatic issuer wake
- **Mailbox FIFO** with send/receive/block-on-empty and per-unit inbox delivery
- **Signal registers** with OR-merge semantics
- **Block/wake transitions** via runtime-side status overrides
- **Scenario test harness** with deterministic replay assertions, golden trace pinning, and invariant checks
- **Fake ISA** (8 opcodes) as a clean-room runtime probe

### Comparison harness

- **Normalized observation schema** shared between CellGov and external oracles
- **Comparison modes**: strict (outcome + memory + events), memory-only, events-only, prefix
- **Classification**: match, divergence (with first-differing byte/event), unsupported, unsettled oracle
- **Multi-baseline comparison**: checks oracle agreement before comparing CellGov
- **Golden snapshot save/load** for regression testing without RPCS3
- **RPCS3 adapter**: TTY-based result extraction via CGOV wire protocol
- **CellGov adapter**: determinism guard (double-run), event normalization from binary trace
- **Human and JSON report formatting**

### Microtest corpus

Six PSL1GHT-compiled C microtests targeting RPCS3's managed SPU thread groups:

| Test | What it proves |
|------|---------------|
| spu_fixed_value | SPU writes a known value via DMA put |
| mailbox_roundtrip | PPU-to-SPU mailbox send, SPU transforms and DMA puts result |
| dma_completion | 128-byte DMA put with tag wait, status header |
| atomic_reservation | SPU getllar/putllc (load-linked, store-conditional) |
| ls_to_shared | Dependent LS store-to-load chain published via DMA |
| barrier_wakeup | Two SPU threads, inter-SPU ordering via shared memory polling |

Each test has interpreter and LLVM baselines (oracle settled -- both decoders agree).

## Crate layering

The workspace is a strict layered dependency DAG. Foundational crates sit at the bottom; consumers at the top. Only direct internal dependencies are shown.

```mermaid
graph BT
  cli["apps/cellgov_cli"]
  compare[cellgov_compare]
  testkit[cellgov_testkit]
  core[cellgov_core]
  exec[cellgov_exec]
  trace[cellgov_trace]
  sync[cellgov_sync]
  dma[cellgov_dma]
  effects[cellgov_effects]
  mem[cellgov_mem]
  event[cellgov_event]
  time[cellgov_time]

  time --> mem
  time --> event

  event --> sync
  mem --> dma
  event --> dma

  mem --> effects
  event --> effects
  sync --> effects
  dma --> effects

  effects --> exec
  effects --> trace

  exec --> core
  trace --> core
  sync --> core
  dma --> core

  core --> testkit

  event --> compare
  testkit --> compare
  trace --> compare

  compare --> cli
  testkit --> cli
  trace --> cli
```

External dependencies are minimal: `serde`, `serde_json`, and `toml` in `cellgov_compare` and `cellgov_cli`. All other crates are dependency-free beyond the workspace.

For per-crate responsibilities and module layout, run `cargo doc --no-deps --open` and read the crate-level doc comments.
