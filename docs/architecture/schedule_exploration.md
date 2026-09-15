# Schedule exploration

`cellgov_explore` enumerates legal alternate schedules without
modifying the runtime, and classifies each outcome as
`ScheduleStable`, `ScheduleSensitive`, or `Inconclusive`. Two searches
reach that verdict, both replaying through a `PrescribedScheduler`
within configurable `max_schedules` and `max_steps_per_run` bounds.

The bounded enumerator records every branching point from a baseline
run and tries each alternate at each one, pruning a pair of units
whose steps never conflict. The backtrack-set search instead builds
happens-before over the events one execution retired, takes the races,
and replays one schedule per race with the later event's unit forced
at the earlier event's step. Neither is optimal: a class can cost more
than one execution in both. They exist together because they share no
reduction, so they agree on the set of final memory hashes a workload
can reach even where they disagree on what reaching it costs -- and a
reduction that drops a class shows up as a disagreement rather than as
a smaller count.

```mermaid
flowchart TD
  base["baseline run"] --> bp["record every branching point"]
  bp --> alt["candidate alternate schedule"]
  alt --> fp{"any step of the two units conflict?"}
  fp -->|"no: provably independent"| prune["pruned, not replayed"]
  fp -->|yes| replay["replay through PrescribedScheduler within max_schedules / max_steps_per_run"]
  replay --> hash["multi-space committed-memory hash (plus named regions under explore_with_regions)"]
  hash --> cls{"across explored schedules"}
  cls -->|all identical| stable["ScheduleStable"]
  cls -->|two differ| sens["ScheduleSensitive"]
  cls -->|"a bound, a refusal, or a baseline that committed nothing"| inc["Inconclusive"]
```

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
