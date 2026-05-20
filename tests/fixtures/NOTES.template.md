---
content_id: <PSN content id, e.g. NPUA80068>
title: <display name>
year: <first release year>
developer: <developer credit, matches docs/titles/<id>.toml>
engine: <engine name, e.g. PhyreEngine / Unreal 3 / "<studio> proprietary">
distribution: <PSN HDD | Retail HDD | Disc ISO>
checkpoint: <ProcessExit | FirstRsxWrite | Pc=0x...>
steps: <retired step count at the checkpoint, integer>
convergence: <Yes | No (<reason>)>
byte_parity: <equivalent | N non-semantic | M non-semantic + N pending | -->
---

# Cross-runner NOTES.md template

Hand-authored prose context for one fixture. Copy this skeleton
into `tests/fixtures/<content-id>/cross_runner/NOTES.md`, fill in
the frontmatter, and keep whichever body sections apply.

The generator never reads or writes NOTES.md. The machine-readable
surface is the triple `compare_report.txt` / `REPRODUCTION.md` /
`cross_runner_summary.json`; this file is for the per-cluster
hypotheses and the next-step plan that do not belong in generated
output.

Keep entries terse: state the fact, name the successor, do not
narrate history.

## Skeleton

```markdown
---
content_id: ...
title: ...
year: ...
developer: ...
engine: ...
distribution: ...
checkpoint: ...
steps: ...
convergence: ...
byte_parity: ...
---

<Title display name> <converges|does not converge> with RPCS3 at
<checkpoint kind>[ (step <N>)]. <One additional sentence on
same-runner determinism, anchor stability, or the convergence-
failure shape.>

## Classifier coverage (per-class)         [omit if not converged]

- `<ClassName>`: <N> bytes. <One-line description of what the
  bytes are and the structural mechanism that classifies them.>

## Unclassified residual (<N> bytes, <M> runs)  [omit if zero]

- `<region>@0x<offset>..0x<end>`: <N> bytes. <Shape: what the run
  looks like in the comparator output and how same-runner reruns
  treat it.> <Hypothesis: best current guess for what the bytes
  encode.> Successor: <name of the investigation or new
  DivergenceClass that would cover this cluster>.

## Next step                              [omit if fully resolved]

<What has to change before the next regen: a refreshed observation
pair at a shared deterministic checkpoint, a CellGov-side fix, a
new structurally-grounded DivergenceClass, etc.>
```

## Notes on each section

- **Frontmatter** mirrors fields already in `docs/titles/<id>.toml`
  + `cellgov/boot_summary.json` + `cross_runner_summary.json`.
  Duplicates the headline so a reader scanning multiple NOTES.md
  files gets the title at a glance without opening the generated
  triple.
- The opening prose line restates the convergence headline in
  human terms. Match the wording the matrix renders.
- **Classifier coverage** appears only when convergence is `Yes`
  and at least one byte classified. Each bullet matches one entry
  in `cross_runner_summary.json`'s `per_class_bytes`.
- **Unclassified residual** appears only when
  `unclassified_bytes > 0`. One bullet per cluster (group adjacent
  unclassified runs that share a hypothesis). Always name a
  successor; an entry without a successor is signal that the
  cluster needs more investigation before it belongs in NOTES.md.
- **Next step** appears whenever the fixture is not in its final
  "Convergence Yes / Byte parity equivalent" state. State the
  specific blocker, not a general aspiration.
