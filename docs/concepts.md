# Concepts

The vocabulary you need to read the rest of CellGov's docs without
being surprised by contradictions. Read this before [titles.md](titles.md)
or [architecture.md](architecture.md).

Six ideas. Thirty minutes.

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
The three titles in the current matrix all reach a checkpoint
(flOw at ProcessExit, WipEout HD Fury and Super Stardust HD at
their first RSX writes or the fault downstream of it); whether
they CONVERGE with RPCS3 at that checkpoint is a separate
question, addressed in the convergence sections below.

The point of a checkpoint is: at this specific deterministic event,
capture the observable state, stop the run, emit the observation.
Two runs of the same title to the same checkpoint produce
byte-identical observations. That is the determinism anchor.

## The null backend: honest vs contaminating divergence

CellGov's central claim is fidelity: every observable the
guest sees should match what a real PS3 (or RPCS3, as the
closest faithful reference) would produce. The corpus of
syscalls a loaded PRX exercises is large; not all of them
are modeled yet. CellGov's policy for the unmodeled gap is
the **null backend**: every syscall a loaded PRX makes that
CellGov has not modeled yet returns an ABI-honest,
per-syscall, traced "not implemented" response (typically
`CELL_ENOSYS` for routes without a specific contract,
`CELL_EINVAL` where the RPCS3 reference returns that for
the unknown-input arm, etc.). Never a blanket `CELL_OK`,
never a fabricated success the guest then consumes as truth.

That policy splits cross-runner divergence into two named
kinds:

- **Honest divergence.** CellGov faithfully reports "not
  modeled" via the null backend and diverges from RPCS3
  because RPCS3 has implemented what CellGov has not yet.
  The divergence is traced and the gap is named. Two
  sub-kinds:
  - **Divergent honest gap.** RPCS3 delivers a real result
    where CellGov returns the not-implemented response.
    This is an implementation target: model the syscall and
    the gap closes.
  - **Convergent honest gap.** CellGov matches RPCS3's own
    divergence-from-hardware (both ignore the same flag
    bits, both reject the same input shape with the same
    errno, etc.). The diagnostic fires as a verbose log of
    behavior that matches RPCS3 anyway, and the guest
    proceeds believing something true. Not an implementation
    target; the diagnostic can downgrade to a one-line note
    when a classifier emerges.
- **Contaminating divergence.** CellGov returns a result
  it did not compute -- a fabricated success the guest
  consumes as truth, after which downstream behavior is
  wrong in a way CellGov cannot detect. This is the failure
  mode the null backend exists to make impossible: every
  unmodeled syscall returns an honest "not implemented"
  response, so no contaminating divergence can arise.

State the criterion plainly: **CellGov never fabricates a
result it did not compute.** Unmodeled syscalls get an
ABI-honest, per-syscall, traced "not implemented" response;
modeled syscalls produce the result; nothing in between.

A divergence from RPCS3 is therefore an implementation
target the oracle named, not a failure of the oracle. The
matrix's `No` verdicts are the worklist; the criterion
the matrix enforces is that none of those `No`s are
fabrications.

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
  `equivalent (N non-semantic)`, `DIVERGE semantic at ...`.

## Two independent verdicts: Convergence and Byte parity

A single "pass/fail" verdict conflates two independent questions a
matrix reader actually asks. CellGov's matrix renders them in
separate columns:

- **Convergence**: did CellGov reach the same architectural state
  as RPCS3? Same outcome, same captured regions, same step count
  within tolerance. `Yes` or `No (<reason>)`.
- **Byte parity**: at that state, are the captured memory regions
  byte-identical (modulo classified non-semantic divergences)?
  Defined only when convergence is `Yes`.

A title can converge and still carry classifier backlog: that is a
successful boot with investigation outstanding, not a failure.
A title can fail to converge regardless of byte parity: byte
parity is undefined because the runners never reached comparable
architectural state. Both columns render side by side.

A `No` convergence row from an honest divergence (the null
backend reported not-implemented at one or more syscalls and
the title's boot path consumed the not-implemented response)
is **the oracle working as designed**: the matrix is naming
which syscalls or which firmware paths are the next
implementation target, not flagging a broken title. A `No`
that would indicate contamination (a fabricated success the
guest consumed as truth) cannot arise under the null backend
-- that is the invariant the policy enforces. The matrix is a
frontier map of the unimplemented syscall surface, not a
pass/fail scoreboard.

Byte-parity vocabulary (only meaningful when convergence is `Yes`):

- `equivalent` -- zero divergent bytes.
- `N non-semantic` -- every divergent byte classifies into a
  structurally-grounded class (ELF header reconstruction, GOT
  slot layout, etc.). The bytes differ but the guest cannot tell.
- `M non-semantic + N pending` -- some bytes classified, some
  awaiting a new structurally-grounded class. The pending bytes
  are visible in `compare_report.txt` and `cross_runner_summary.
  json` (`unclassified_runs`). The verdict moves to `equivalent`
  only when a new `DivergenceClass` lands that covers the bytes;
  investigation continues in `NOTES.md`.
- `--` -- byte parity is undefined because convergence is `No`.

Convergence-failure reasons render inline in the matrix:

- `No (outcome: <a> vs <b>)` -- different terminal outcomes
  (Fault, Completed, Timeout, ...).
- `No (region count: <a> vs <b>)` -- different number of captured
  regions.
- `No (region[i] identity: ...)` -- region at index `i` differs by
  name or address.
- `No (region[i] <name> length: <a> vs <b>)` -- same identity,
  different byte length.
- `No (same-runner step mismatch on <runner>: <a> vs <b>)` --
  determinism failure: a single runner produced different step
  counts across reruns.
- `No (step count: <a> vs <b> (tolerance T))` -- cross-runner
  step delta beyond tolerance.

### Worked example: what a converged row would look like

No title in the current matrix converges yet (see
[titles.md](titles.md) -- all three rows render `No (outcome:
... vs Completed)`). This example is therefore illustrative:
it describes what a fixture would carry once a title's
divergent-honest-gap count reaches zero and convergence
becomes possible. Treat it as a teaching shape, not a
present verdict.

A converged row's cross-runner fixture would record:

```
Convergence: Yes
Byte parity: 599 non-semantic + 125 pending
```

Convergence holds: both runners reach the manifest checkpoint
with the same outcome, same captured regions, same step count.

The 599 non-semantic bytes would include addresses like the
low byte of `e_ehsize` (CellGov `0x00`, RPCS3 `0x40`) -- in
the loaded ELF header, which the guest never reads during
execution; classifier rule `ElfHeader` covers it because the
range is derivable from the title's own PHDR table.

The 125 pending bytes would be byte-level divergences the
classifier has no general rule for yet. They would be
enumerated in the fixture with their locator and the
cellgov-side / rpcs3-side bytes; hypotheses about their
source would live in `NOTES.md`. The verdict moves to
`equivalent` (no pending) only when a new structurally-grounded
`DivergenceClass` lands that covers them.

### Why the two-column split matters

A single verdict collapses two independent failure shapes into
one cell. A title that converges with a small classifier backlog
reads the same as a title that does not converge at all -- and a
reader cannot tell which is which without opening the fixture.
The two columns separate the two questions:

- Convergence answers "did CellGov reach where RPCS3 reached?"
  This is the actually-bad-when-No state.
- Byte parity answers "are the captured bytes the same?" This is
  meaningful only when convergence is `Yes`, and a `Pending`
  count is investigation backlog, not regression.

If the matrix silently treated unclassified bytes as `equivalent`
when convergence holds, the project would slide into per-title
compatibility hacks: each new title would arrive with its own
list of "trust me, these bytes are fine" entries. `Pending`
is the honest verdict: bytes differ, no general rule covers
them yet, the fixture is on disk so a human can see exactly
which bytes, and the verdict moves to `N non-semantic` only
after a new structurally-grounded `DivergenceClass` lands.

The verdict vocabulary appears in three places:

- `docs/titles.md` compatibility matrix columns.
- `tests/fixtures/<serial>/cross_runner/compare_report.txt`
  two-line header.
- This document.

`CrossRunnerSummary::display_matrix_columns()` is the source of
truth for the wording. If concepts.md disagrees with the code,
fix concepts.md.

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

If CellGov said only `pass` for every title, the recompiler would
have no way to tell the two categories apart: every byte would
look semantically required. If CellGov said `fail` without further
classification, every title would look broken. The Convergence +
Byte parity split plus the per-byte classification narrative
gives the recompiler (and the reader) exactly the information
needed: did the runners reach the same state, and at that state,
which bytes must be faithful and which are observation-format
accidents.

The vocabulary is not just docs hygiene. It is the interface
contract between CellGov (the oracle) and whatever consumes its
observations (a static recompiler today, a differential debugger
tomorrow, a hand-written reimplementation of a specific title
never).
