# Schedule exploration

`cellgov_explore` enumerates legal alternate schedules without
modifying the runtime, and classifies each outcome as
`ScheduleStable`, `ScheduleSensitive`, or `Inconclusive`. Two searches
reach that verdict, both forcing their choices through a
`PrescribedScheduler` within configurable `max_schedules` and
`max_steps_per_run` bounds. `max_schedules` bounds equivalence
classes.

The optimal search runs one execution per equivalence class. It runs
an execution to a maximal sequence, reads its races, and for each one
records the sequence that reaches the reversed order in a wakeup tree
at the prefix before the earlier event. The next execution takes the
least branch that tree holds. A sleep set carries the units already
explored from a prefix so none is explored twice, and the wakeup tree
is what keeps that sleep set from blocking: it holds enough of an owed
sequence to reach the state the race asked for. This is the search
behind `explore`, `explore_window` and `explore_with_regions`.

The backtrack-set search is the older algorithm, kept beside it. It
walks the same races without the wakeup trees and sleep sets, so a
class can cost it more than one execution. It exists because the two
share no reduction: they agree on the set of final memory hashes a
workload reaches even where they disagree on what reaching it costs,
and a reduction that drops a class shows up as a disagreement rather
than as a smaller count. A smaller count is what a dropped class looks
like to every measurement that does not have a second search to check
against.

```mermaid
flowchart TD
  run["run an execution to a maximal sequence"] --> races["build happens-before, take the races"]
  races --> owe["per race: record the reversing sequence in the wakeup tree at the earlier event's prefix"]
  owe --> back["retire the branch just explored, add its unit to that prefix's sleep set"]
  back --> next{"any prefix still owes a branch?"}
  next -->|yes| run
  next -->|no| hash["compare the multi-space committed-memory hashes (plus named regions under explore_with_regions)"]
  hash --> cls{"across the classes explored"}
  cls -->|all identical| stable["ScheduleStable"]
  cls -->|two differ| sens["ScheduleSensitive"]
  cls -->|"a bound, a refusal, or a baseline that committed nothing"| inc["Inconclusive"]
```

`ScheduleStable` carries a class count when the search covered one
execution per class and hit no bound. A bounded run reports no count
and can only be inconclusive.

The count is narrower than it reads. A reversal that names a unit no
state at that depth can run is dropped and counted, and one drop
withdraws the count for the whole run, so a search can answer for every
outcome and still report none.
[`exhaustive_cover`](../../crates/cellgov_explore/tests/exhaustive_cover.rs)
walks the choice tree of a small workload, finds both committed
memories it can reach, and holds the search against them: it reaches
both, and reports no count.

`StepFootprint`, extracted from the ten shared-resource `Effect`
variants, drives conservative dependency analysis: step pairs with
non-overlapping footprints prune as provably independent, including
DMA destinations against reservation lines in both directions (a DMA
completion clears cross-unit reservations covering its destination
line even when the bytes miss the conditional store's exact range).
A guest load of committed memory emits `SharedReadIntent`, so all
three of Bernstein's intersections hold over data accesses and a
write-read race whose read steers a later store to a disjoint address
conflicts rather than prunes. Two loads of the same bytes still
prune. Instruction fetch records what a PPU block read from the text
region, coalesced into one read per run of addresses at the block
boundary, so a fetch conflicts with another unit's write there. The
dependency
module states what each clause pairs, and carries the argument that
makes each domain rule sound -- why two units waiting on different
barriers cannot interact, why the cross-unit half of the reservation
rule is the half a footprint pair is asked for, and why the granule
arithmetic cannot saturate.

Three things reach committed state without reaching a footprint. Guest
time is the first: one global clock advances per step, and a step that
touches no shared resource still moves it. A PPU `mftb` reads that
clock straight into a guest register, and a timer deadline fires from
it. The relation records neither, so two steps it calls independent
can commit different memory when they swap. One clock reader it does
record is a transfer's landing tick. A step carries what each transfer
in flight during it will touch at completion -- its destination, and
its source unless an inline payload already holds the bytes -- and
those ranges conflict with another step's access to them. A step that
touches the bytes of a transfer in flight during itself conflicts with
every step instead, because every step carries ticks and so decides
which side of the landing that step falls on. The second is the RSX FIFO advance
pass, whose effects commit guest memory and sweep reservations from a
batch no unit's step emitted. The third is the LV2 handler surface: a
footprint reads one unit's own step effects, and an LV2 handler's
commit through `Runtime::host_write` belongs to no unit's step.

A park the commit pipeline takes from the step result rather than an
effect does reach a footprint. The independence relation reads the
yield reason for it, so a wake of the parked unit conflicts with the
step that parked it rather than pruning against it.

`Execution` carries those footprints as events: one per retired step,
identified by the step's position and the unit that ran it. It builds
happens-before over that sequence from three orders, transitively
closed through one clock vector per event:

- program order: the order one unit ran its own events;
- conflict order: the order the schedule ran two events whose
  footprints conflict;
- a wake before the step it released, which no footprint pair reaches
  because the wake names a unit and the released step emits no wait.

`Execution::races` then reads the relation for the pairs nothing but
their own conflict order holds apart. An edge added here removes a
race, so it removes a reversal the search would have owed: the
relation is the one place in the module where erring toward more order
costs cover rather than budget.

Schedules compare through the multi-space committed-memory hash, so
divergence confined to a spawned child's address space is witnessed.
`explore_with_regions` also captures named memory regions per
schedule for comparison against external baselines; a region spec
that fails to resolve is captured unresolved and fails oracle
comparison loudly instead of matching zeros.
