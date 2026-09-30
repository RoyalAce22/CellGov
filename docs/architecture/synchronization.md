# Synchronization

This file covers how the LV2 primitives park and wake PPU threads,
and the atomic reservation model the PPU and SPU share. Each
handler must read and mutate atomically within its own dispatch,
and the runtime commits each dispatch atomically.

## Synchronization primitives

The LV2 primitives park and wake PPU threads under one
contract. Each has its own table inside `Lv2Host`, keyed by the
guest-visible id:

- `LwMutexTable`
- `MutexTable`
- `SemaphoreTable`
- `EventQueueTable`
- `EventFlagTable`
- `CondTable`

Every table composes the shared `WaiterList`, a strict FIFO queue
of `PpuThreadId`. Every table keeps its objects in a lane map whose
partial enters `sync_state_hash`; an empty table contributes
nothing.

### Block / wake protocol

**Park side.** A wait handler whose predicate fails parks the
caller. The predicate fails when:

- the mutex is owned;
- the semaphore count is zero;
- the event queue is empty;
- the event flag mask is unsatisfied;
- the call is a cond wait (unconditionally).

The handler then:

- enqueues the caller on the primitive's waiter list;
- records a `PendingResponse` for that unit in the runtime's
  `SyscallResponseTable`;
- returns `Lv2Dispatch::Block { reason, pending, effects }`.

The runtime moves the unit `Runnable -> Blocked` and applies the
effects atomically.

- `reason` contains the `Lv2BlockReason` (primitive id plus, for
  cond, the associated mutex id).
- `pending` contains what the wake resolver needs to complete the
  syscall: a return code, a 32-byte event payload, a u64 flag
  observation, or a cond-reacquire marker.

**Release side.** An unlock / post / send / set / signal handler
reads the waiter list and the primitive state in one dispatch,
decides the wake, and returns
`Lv2Dispatch::WakeAndReturn { code, woken_unit_ids,
response_updates, effects }`. The runtime then:

1. sets the releaser's r3 to `code`;
2. applies the per-waiter `response_updates`;
3. walks `woken_unit_ids`: for each it takes the pending response,
   writes the r3 / out-pointer effects, and moves the unit
   `Blocked -> Runnable`.

A response update can swap a waiter's `PendingResponse` wholesale:

- event queue send uses it to deliver the payload;
- event flag set uses it to deliver the observed bit pattern;
- cond signal uses it to swap `CondWakeReacquire` for `ReturnCode`
  on clean acquire.

**Continuation pointers.** Continuation pointers (event queue out
pointer, event flag result pointer) live on the primitive's waiter
entry, not on the release-side dispatch.

*Why:* parking records everything the wake needs, which rules out
the lost-wake class of bugs.

```mermaid
sequenceDiagram
  participant W as Waiter (PPU unit)
  participant H as Lv2Host
  participant R as Runtime
  participant S as Releaser (PPU unit)
  W->>H: wait syscall, predicate fails
  H->>H: enqueue W on the WaiterList, record a PendingResponse
  H-->>R: Lv2Dispatch Block (reason, pending, effects)
  R->>W: Runnable to Blocked, effects applied atomically
  S->>H: unlock / post / send / set / signal
  H->>H: read the waiter list and primitive state, decide the wake
  H-->>R: WakeAndReturn (code, woken_unit_ids, response_updates, effects)
  R->>S: r3 = code
  R->>W: apply the response update, write r3 and out-pointers, Blocked to Runnable
```

### Process-exit purge

Process exit removes every thread the process owns from every
waiter list in one sweep. The sweep covers every primitive table
plus the per-thread join-waiter lists.

*Why:* exit finishes all the process's units, so any later grant to
one is a resource no thread will consume or release:

- a semaphore count sunk into a dead waiter;
- a mutex transferred to an owner that can never unlock it;
- an exit value delivered to a joiner that no longer runs.

The purge is record-only: nothing is woken and no pending-response
entry is cleared.

*Why:* the purged threads are finished, not resumed.

Per-primitive purge counts are witnessed.

A mutex still owned by a dead thread keeps its owner and is
reported as a named invariant break.

*Why:* reclaiming it needs creator attribution the shared-object
namespace does not record. The process-shared case with surviving
attachments has no oracle. So retained-locked is the honest answer
for both.

### Cond-wake re-acquire (two-hop block)

`sys_cond_wait` alone breaks the straight release-wakes-caller
pattern: on wake the caller needs the associated mutex re-held.
The handler returns `Lv2Dispatch::BlockAndWake`.

*Why:* releasing the mutex on the way into the cond wait can
transfer ownership to a parked mutex waiter. That waiter must wake
in the same dispatch the cond caller blocks.

On the signal side, the wake target has
`PendingResponse::CondWakeReacquire { mutex_id, mutex_kind }`.
The signal handler consults the mutex table:

- Unowned: acquire on the waker's behalf, swap the pending
  response to `ReturnCode { code: 0 }`, and include the waker
  in `woken_unit_ids` -- a classic wake: r3 = 0, back to
  Runnable, mutex held.
- Held: re-park the waker on the mutex waiter list, swap the
  pending response to `ReturnCode { code: 0 }`, and leave the
  waker Blocked. When the holder calls `sys_mutex_unlock`, the
  unlock-wake path transfers ownership to this waker and
  resolves the swapped pending response.

Cond is non-sticky: a `sys_cond_signal` / `_signal_all` /
`_signal_to` on a cond with no waiters is observably lost. No
pending-signal counter is kept; a lost signal leaves the cond
table's state hash unchanged.

```mermaid
stateDiagram-v2
  state "Blocked on the cond" as OnCond
  state "Blocked on the mutex (pending ReturnCode 0)" as OnMutex
  Running --> OnCond : sys_cond_wait releases the mutex, BlockAndWake may hand it to a parked mutex waiter
  OnCond --> Running : signal with the mutex unowned, acquired for the waker, r3 = 0
  OnCond --> OnMutex : signal with the mutex held, re-parked on the mutex waiter list
  OnMutex --> Running : holder unlocks, ownership transfers, pending response resolves
```

### Lost-wake prevention

The classic lost-wake bug is a race between
check-waiter-list-then-wake and check-count-then-decrement.
CellGov's runtime runs on one OS thread and drives every guest
execution unit (PPU and SPU) sequentially through the [step
loop](runtime_pipeline.md#per-step-pipeline), one committed
batch at a time. So two guest threads never run at once on two
host cores, and no handler preempts another mid-read.

Guest units still interleave, but only at commit boundaries and in
a reproducible order. So each handler must still read and mutate
atomically within its own dispatch:

- Park handlers read primitive state AND install the block in
  the same dispatch.
- Release handlers read the waiter list AND drain it in the
  same dispatch.

The runtime commits each dispatch atomically.

**Regression coverage.** One post-before-wait test per non-cond
primitive asserts that a release scheduled before the wait
observably unblocks the waiter. For cond the inverse holds:
a signal-before-wait must NOT wake a later waiter, tested
against all three signal variants.

## Atomic reservation model

PPU `lwarx` / `stwcx.` / `ldarx` / `stdcx.` and SPU
`MFC_GETLLAR` / `MFC_PUTLLC` share one reservation model. The
granule is 128 bytes (a Cell BE cache line) on both sides.

**The SPU commands identify a line by any byte inside it.**
Alignment is not checked for the atomic commands, so a misaligned
effective address refuses nothing:

- `MFC_GETLLAR` delivers the containing line to local store and
  reserves that line;
- `MFC_PUTLLC` stores over the containing line.

The bytes and the reservation always cover the same memory.

A refused `MFC_GETLLAR`:

- reports the line it could not fetch;
- drops the reservation register rather than taking a line it
  never read;
- leaves the atomic status alone, so the status keeps reporting
  the last command that completed.

A completed one reports the getllar bit of `MFC_RdAtomicStat`;
`MFC_PUTLLC` reports its own success bit.

**Two pieces of state.** Every execution unit has a local
register -- `Option<ReservedLine>` on `PpuState` / `SpuState`. An
atomic load sets it. It is cleared by:

- a conditional-store retirement;
- an SPU's own put or unconditional lock-line put over the line;
- another unit's write to the line.

The holder's other stores leave it in place.

The committed cross-unit view is `cellgov_sync::ReservationTable`,
a unit-ordered map from `UnitId` to `ReservedLine` owned by the
commit pipeline. Its lanes enter `sync_state_hash` with mailboxes,
signals, LV2 host state, and syscall responses.

**Verdict rule.** `stwcx.` / `stdcx.` / `MFC_PUTLLC` succeed
when BOTH the local register is set AND its line matches the
store's line.

The committed half of the check is a step-start refresh. At the top
of every `run_until_yield`, a local register that is `Some` while
`ExecutionContext::reservation_held(unit_id)` is false is cleared.
That state means a cross-unit write in an earlier commit cycle
cleared the entry.

Intra-step verdicts trust the local register alone.

*Why:* the context is frozen during the step.

```mermaid
flowchart TD
  ld["lwarx / ldarx / MFC_GETLLAR"] -->|Effect ReservationAcquire| tbl["ReservationTable entry (committed)"]
  ld --> loc["local register = Some(line)"]
  wr["any committed write from another unit or the host: SharedWriteIntent, ConditionalStore, host write"] -->|clear_covering| tbl
  start["step start: local is Some but reservation_held is false"] --> clr["local register cleared"]
  st["stwcx. / stdcx. / MFC_PUTLLC"] --> v{"local Some AND line matches the store?"}
  v -->|no| nope["conditional store fails"]
  v -->|yes| src{"MFC_PUTLLC only: does the 128-byte source reach local store?"}
  src -->|no| ref["SPU fault, no effect emitted"]
  src -->|yes| ok["Effect ConditionalStore: bytes commit, own entry dropped, clear sweep on the other entries"]
```

**Effect vocabulary.** Two `Effect` variants drive the table:

- `ReservationAcquire { line_addr, source }` inserts or replaces
  the unit's entry. Emitted by `lwarx` / `ldarx` (PPU) and
  `MFC_GETLLAR` (SPU). A `MFC_GETLLAR` whose line does not reach
  local store emits none and clears the local register too, so the
  two halves agree that the unit holds nothing.
- `ConditionalStore { range, bytes, source, ordering,
source_time }` commits the success path of `stwcx.` / `stdcx.`
  / `MFC_PUTLLC`. The commit pipeline:
  - applies the bytes through the normal staging / drain path;
  - drops the emitter's own reservation entry;
  - runs the clear sweep against all other entries covering the
    line.

  On the PPU side the conditional store goes through the store
  buffer. So its effect is emitted in program order with the
  block's plain stores and ahead of any later `ReservationAcquire`
  in the same block. A full buffer retries the instruction next
  block with CR0 and the reservation untouched.

**Clear-sweep contract.** Every write path that commits bytes to
main memory fires the clear sweep. The lost-reservation
regression suite in `cellgov_core::tests::runtime_tests` pins
the invariant per path. The sweep fires from three paths:

1. `SharedWriteIntent` commit, in the commit pipeline's apply
   pass after the staging drain.
2. `ConditionalStore` commit, same as (1) via the shared
   byte-deposit path, plus the emitter's own entry is dropped.
3. A host write, through `Runtime::host_write`.

The host writes through `Runtime::host_write` are:

- LV2 dispatch effects;
- wake and out-parameter payloads;
- DMA completion;
- the RSX control-register and flip-status mirrors;
- shared-view seeding and fanout;
- the placements the driving program makes itself through
  `Runtime::place_bytes`.

Validation, the sweep and the trace record sit together in
`Runtime::host_write`.

*Why:* a new host-side write then inherits the sweep instead of
making its own call.

A write the host makes on one unit's behalf exempts that unit, as
(1) exempts its emitter. A mechanism with no unit behind it exempts
nobody.

**Scope and bounds.** The reservation table and local registers
are the full contention model.

Memory-barrier instructions decode as their own instructions and
change no result. They are:

- PPU: `sync`, `lwsync`, `ptesync`, `eieio`, `isync`;
- SPU: `sync`, `sync.c` and `dsync`.

*Why:* each unit runs its accesses in program order and each step
commits as one batch, which orders more than any barrier does.

A traced run records each retired barrier as a `Barrier` record
(unit, address, kind). So a consumer that runs the program on
reordering hardware has the location of every barrier instruction
the run retired.

The SPU's MFC ordering commands (`mfcsync`, `mfceieio`, `barrier`)
and the fence and barrier forms of put, get and sndsig record the
same way when they queue. Unlike the instructions, they also order
the MFC command queue, which completes commands out of order.

[Schedule exploration](schedule_exploration.md) over contention
workloads is conservative: `StepFootprint::reservation_lines` marks
a step dependent on any other step whose writes cover the reserved
line.
