# Concepts

The vocabulary you need to read the rest of CellGov's docs without
being surprised by contradictions. Read this before [titles.md](titles.md)
or [architecture.md](architecture.md).

Five ideas. Thirty minutes.

## What CellGov produces: observations

CellGov runs PS3 PPU and SPU code deterministically. It does not
render, play audio, or execute at real-time speed. What it produces
is a structured record of everything the guest did that is
observable to anyone outside the CPU: memory writes, syscall
arguments and returns, register state at chosen stopping points,
the ordered stream of kernel events.

That record is an **observation**. Observations are typed, JSON-
serialisable, and independent of which runtime produced them.
CellGov emits observations. RPCS3 (via a patched TTY dump hook and
the `rpcs3_to_observation` bridge) emits observations. A future
static recompiler can emit observations. Anything that can be
compared against a PS3 can be described as an observation.

Observations are not traces. A trace is the full step-by-step
execution history; an observation is a snapshot at a chosen
stopping point plus the ordered stream of events that led there.
Traces are used for divergence localization; observations are used
for cross-implementation agreement.

## Checkpoints: where an observation stops

A PS3 game runs forever from CellGov's perspective: there is no
natural "done" for an interpreter. Observations stop at a
**checkpoint** -- a deterministic event CellGov recognises as a
useful capture point.

Different titles need different checkpoints. A title that calls
`sys_process_exit` during its startup-probe stage has a natural
stopping point at the exit call. A title that proceeds into
rendering has no exit; it stops at its first write to the RSX
command register, which is the earliest point that is both
deterministic and post-boot-useful.

The two checkpoints CellGov currently recognises:

- **`ProcessExit`** -- the guest called `sys_process_exit`. Used
  for titles that reach a natural shutdown during the captured
  window.
- **`FirstRsxWrite`** -- the first PPU write to guest address
  `0xC0000040` (the RSX control register put-pointer, typically
  inside `_cellGcmInitBody`). Used for titles that proceed past
  init into an RSX command stream. SSHD currently stops here.

A third checkpoint kind, `PcReached`, stops at a specific PC
address and exists for manifest-driven frontier exploration, not
for the default compatibility matrix. The `[rsx] mirror = true`
manifest flag changes what "past FirstRsxWrite" means for a title
by mapping the RSX region read/write so the put-pointer write
lands instead of tripping the checkpoint.

A title that exits its budget cap without hitting any of the above
has no checkpoint observation; cross-runner comparison for it is
queued pending boot advancement to a deterministic stopping point.
flOw and WipEout HD Fury are in that state today.

The point of a checkpoint is: at this specific deterministic event,
capture the observable state, stop the run, emit the observation.
Two runs of the same title to the same checkpoint produce
byte-identical observations. That is the determinism anchor.

## Cross-runner comparison

CellGov is one implementation of "what should the PS3 have done."
RPCS3 is another. They execute different code, use different
memory layouts, wake threads in different orders. When both run
the same ELF to the same checkpoint and their observations agree,
that is **cross-runner agreement**: the same program-level answer
derived by two independent paths.

The comparison tool (`cellgov_cli compare-observations`) walks
both observations field by field. Any difference in outcome,
memory bytes, or event sequence produces a `DIVERGE` line naming
the first divergent field. If every field matches exactly, the
tool prints `MATCH`.

`MATCH` is strong evidence that CellGov's model is correct for the
code path the title exercised up to that checkpoint. `DIVERGE` is
the starting point for investigation -- not a conclusion.

## Semantic vs non-semantic divergence

This is the section the rest of the docs are read through. Skip it
and the compatibility matrix will look self-contradictory.

Two independent PS3 implementations can produce byte-different
observations without disagreeing about what the program did. The
bytes differ; the program behaviour does not.

- The ELF-in-memory layout includes metadata bytes the guest never
  reads.
- HLE stub slots are laid out differently between CellGov and
  RPCS3 because the two pick a different (but equally valid)
  iteration order over a module's NID table.
- SELF reconstruction (decrypting the signed-encrypted PS3
  executable format into a regular ELF) can land zero-initialised
  bytes in header slots CellGov does not populate but RPCS3 does.

None of those differences change what the guest CPU observes when
it executes. But the byte-level compare tool does not know that;
it reports `DIVERGE` because bytes differ.

Distinguishing between these two layers is the entire point of
this section:

- **Raw divergence** (what the tool prints): bytes differ. Layer:
  byte-for-byte equality. Vocabulary: `DIVERGE at byte 0x17`,
  `MATCH`.
- **Classified divergence** (what a human decides after looking):
  each byte difference is either **semantic** (the program would
  behave differently on a real PS3) or **non-semantic** (the byte
  lives in an address no guest code reads, or encodes metadata the
  guest doesn't act on). Layer: behaviour equivalence. Vocabulary:
  `equivalent (N bytes non-semantic)`, `DIVERGE semantic at ...`.

A title **passes** cross-runner when every byte-level divergence is
classified non-semantic. It is marked `equivalent (N bytes
non-semantic)` in the compatibility matrix. The raw report still
shows `DIVERGE` lines because bytes did differ; the verdict line
says `Verdict: equivalent` because the differences did not matter.

There is no contradiction in "DIVERGE at byte 0x17" sitting next to
"Verdict: equivalent". The first is a byte-level fact. The second
is a behaviour-level judgement backed by the classification
narrative in the same file. They are statements at different
layers.

### Worked example: Super Stardust HD

SSHD's cross-runner report against RPCS3 contains exactly one
line:

```
DIVERGE region code: first byte differs at offset 0x35 (guest 0x10035) -- 00 vs 40
```

One byte. At guest address `0x00010035`, CellGov has `0x00` and
RPCS3 has `0x40`. Byte-equality check: DIVERGE.

What lives at `0x00010035`? It is the low byte of the ELF
header's `e_ehsize` field, written when the loader maps PT_LOAD
#0 starting at `p_offset = 0`. The spec value is `0x40` (= 64,
the standard ELF64 header size). RPCS3 keeps the bytes from the
SELF in place; CellGov's `cellgov_firmware decrypt-self` clears
this slot during reconstruction.

The guest never reads its own ELF header during execution. The
loader uses the field to know how big the header is, but that
read happens on the host side before the program counter ever
points into the header range. Same program. Same execution.
Different byte at `0x00010035`.

**Verdict: equivalent (1 byte non-semantic).**

The verdict sits on top of the byte-level DIVERGE. Both are true
statements at different layers.

### Why the verdict term matters

If the compatibility matrix said `MATCH` for a title with any
non-zero divergence, that would be wrong: bytes actually differ.
If it said `DIVERGE` for one classified-non-semantic byte, that
would read as failure at a glance even though the program
behaves identically. `equivalent` threads the needle: it
concedes the bytes differ, and asserts the behaviour does not.

The same term appears in three places:

- `docs/titles.md` compatibility matrix column.
- `tests/fixtures/<serial>_cross_runner/compare_report.txt`
  classification verdict line.
- This document.

When a title's classification changes (new divergence class
discovered, previously non-semantic reclassified as semantic), all
three get updated together. Consistency across those three surfaces
is how a reader avoids whiplash.

## Why this matters for static recomp

CellGov exists to be the oracle layer for static recompilation of
PS3 games to native binaries. A static recompiler reads CellGov
observations as ground truth and produces a native binary whose
execution, fed the same inputs, would produce the same
observations.

The recompiler must distinguish:

- **Semantic bytes** -- memory the program reads and acts on. The
  recompiler MUST preserve these or its output is wrong.
- **Non-semantic bytes** -- memory the program never reads, or
  metadata the program does not act on. The recompiler MAY
  represent these differently (or not at all) because its output
  is a native binary, not a PS3 memory image.

If CellGov said `MATCH` for every title, the recompiler would have
no way to tell the two categories apart: everything would look
load-bearing. If CellGov said `DIVERGE` without classification,
every title would look broken. The `equivalent (N bytes
non-semantic)` verdict plus the per-byte classification narrative
gives the recompiler (and the reader) exactly the information
needed: which bytes must be faithful, which bytes are observation-
format accidents.

The vocabulary is not just docs hygiene. It is the interface
contract between CellGov (the oracle) and whatever consumes its
observations (a static recompiler today, a differential debugger
tomorrow, a hand-written reimplementation of a specific title
never).
