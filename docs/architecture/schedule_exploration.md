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

`ScheduleStable` means no schedule diverges, not merely that no
sampled one did, whenever the result carries a class count. The count
is present only when the search covered one execution per class and
hit no bound; a bounded run reports none and can only be inconclusive.

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
prune, and instruction fetch emits nothing, so a fetch still prunes
against another unit's write to the text region. The dependency
module states what each clause pairs, and carries the argument that
makes each domain rule sound -- why two units waiting on different
barriers cannot interact, why the cross-unit half of the reservation
rule is the half a footprint pair is asked for, and why the granule
arithmetic cannot saturate.

Three things reach committed state without reaching a footprint.
Instruction fetch is the first, as above. Guest time is the second:
one global clock advances per step, a DMA completion lands at the
first commit whose clock reached its completion tick, and a PPU `mftb`
reads that clock straight into a guest register, so a step that
touches no shared resource at all still moves an in-flight transfer
relative to every later step. Two steps the relation calls
independent can therefore commit different memory when they swap;
[`shared_clock`](../../crates/cellgov_explore/tests/shared_clock.rs)
holds the witness. The third is the RSX FIFO advance pass, whose
effects commit guest memory and sweep reservations from a batch no
unit's step emitted.

`Execution` carries those footprints as events: one per retired step,
identified by the step's position and the unit that ran it. It builds
happens-before over that sequence -- two events of one unit ordered by
the order the unit ran them, two conflicting events by the order the
schedule ran them, transitively closed through one clock vector per
event -- and `Execution::races` reads the relation for the pairs
nothing but their own conflict orders.

Schedules compare through the multi-space committed-memory hash, so
divergence confined to a spawned child's address space is witnessed.
`explore_with_regions` also captures named memory regions per
schedule for comparison against external baselines; a region spec
that fails to resolve is captured unresolved and fails oracle
comparison loudly instead of matching zeros.
