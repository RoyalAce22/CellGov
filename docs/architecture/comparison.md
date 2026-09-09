# Comparison harness

`cellgov_compare` reduces a run of any runner (CellGov, RPCS3, future
recompiled output) to a normalized `Observation`: outcome, named
memory regions, ordered events, optional state hashes, runner
metadata, and the identity triple the run was composed from. Each named region carries its `AddressSpaceId` (space 0 for
the boot process, spawned children numbered from 1 in spawn order;
see [guest_memory.md](guest_memory.md#per-process-address-spaces)),
so a checkpoint manifest can observe a child's memory; RPCS3 captures
hold space 0 only, and the bridge refuses a manifest naming any
other. The comparison layer diffs two observations field by field in
four modes: strict (outcome + memory + events), memory-only,
events-only, prefix. Under every mode, two observations from the same
runner that both carry CellGov state hashes must agree on them: a
saved CellGov baseline whose hashes no longer match a fresh run of the
same scenario is a divergence, so a baseline captured under an older
hash construction fails instead of re-blessing itself. Cross-runner
pairs never compare hashes (the RPCS3 adapter records none).
Multi-baseline mode checks oracle agreement across, e.g., the RPCS3
interpreter and LLVM before declaring a CellGov divergence.

For long boot snapshots, `observe_from_boot` builds observations
from `boot run` outputs; `cellgov diff observations` reads
two JSON files and reports MATCH or the first differing field. The
determinism check requires two CellGov runs of the same ELF to
produce byte-identical observations.

Which firmware answered a run, and which of a title's installed
versions it composed, travel with the observation rather than beside
it. Every comparator prints both sides' triples before its verdict and
says out loud when the two differ, because a divergence between two
differently-composed runs is a difference between versions until it is
shown otherwise. The triple is context, not a verdict: a mismatch
never drives an exit code on its own. The game half carries the
title's version under the `PARAM.SFO` key its tree named it by, so a
warning names the key beside the value. State traces carry the same
identity as a fixed-width fingerprint in their header record, so
`diverge` reports a cross-triple scan from the stream alone. An
artifact naming no triple was written before the store carried
versions, or by a runner that reports none; absence never reads as a
mismatch.

A runner CellGov does not compose for names no triple, because no store
entry describes it. It still names a firmware. Its capture carries
`runner_firmware`, the version read out of that runner's own
installation -- the guest-path mapping in its configuration, then the
console version file in whichever tree that mapping names. The read
happens where the capture becomes an observation, not where a report is
later generated: a runner's installation can change between the two, and
a version read late names a library the capture never saw. A capture
whose conversion was given no installation to read names no version, and
a verdict built from it is refused rather than assumed.

A byte-parity verdict is a statement about two runs of one firmware
library, so `CrossRunnerSummary` carries the cell triple and the other
runner's version together. Three shapes are refused on load rather than
rendered: the two versions disagreeing, either side named alone, and a
file whose recorded firmware differs from the cell its directory names.
A summary naming neither side predates the schema and makes no claim to
contradict. A cross-firmware comparison remains a legitimate experiment;
what it is not is a parity verdict, and nothing renders it as one.

```mermaid
flowchart LR
  man["checkpoint manifest (regions, spaces)"] --> cg
  man --> br
  cg["cellgov boot run --save-observation"] --> oc["Observation JSON (CellGov)"]
  r3["patched RPCS3, oracle-mode config"] -->|"CELLGOV_DUMP_PATH / CELLGOV_DUMP_REGIONS"| dump["binary dump at _sys_process_exit"]
  dump --> br["rpcs3_to_observation --config-hash"]
  br --> orr["Observation JSON (RPCS3)"]
  oc --> cmp["diff observations (field by field) / compare --against-baseline (strict, memory, events, prefix)"]
  orr --> cmp
  cmp --> out["MATCH, or the first differing field"]
```

## Per-step divergence localization

Two scanners turn per-step state-trace files into diff reports:

- `cellgov_compare::diverge(a, b)` walks two trace byte buffers,
  filters each to `PpuStateHash` records, and reports the first
  index where they disagree: the first *scalar-visible*
  disagreement, per the [per-step coverage caveat](runtime_pipeline.md#effects-and-trace-records).
  Four outcomes: `Identical { count }`,
  `LengthDiffers { common_count, a_count, b_count }`,
  `Differs { step, a_pc, b_pc, a_hash, b_hash, field }` with `field`
  in `{Pc, Hash}`, or `CorruptTrace { common_count, a_error, b_error }`
  when a record on either side fails to decode; the last is no
  verdict on the runs, since nothing past the cut was compared.
  Checks run step count -> PC -> hash, so the report names the
  highest-level divergence first. Surfaced via
  `cellgov diff diverge <a.state> <b.state>` (exit 3 on a corrupt
  trace). The scan is linear in record count.
- `cellgov_compare::zoom_lookup(a_zoom, b_zoom, step)` consumes
  separate zoom-trace files (`PpuStateFull` records emitted only
  inside the unit's window) and returns
  `Found { step, a_pc, b_pc, diffs }` with per-field
  `RegDiff { field, a, b }` entries, or
  `MissingStep { step, a_missing, b_missing }`. The snapshot carries
  the full fingerprint input set, so an empty `diffs` means the
  states agree on everything the hash folds; if `PpuStateHash`
  diverged at that step, the harness is skewing snapshots against
  hashes and the scan must not resume past it. Surfaced via
  `cellgov diff zoom <a> <b> <step>`.

`boot run --save-state-trace <path>` writes the runtime's per-step
`PpuStateHash` trace to disk, switching the runtime mode from
`FaultDriven` to `DeterminismCheck` for the run; that file is what
`diverge` and `zoom` consume. With `--patch-byte` for boot-time
memory injection, diffing two CellGov traces (unpatched + patched)
answers "do these N bytes propagate into any tracked PPU register
during the boot?"

```mermaid
flowchart LR
  a["boot run --save-state-trace a.state"] --> dv["cellgov diff diverge a b"]
  b["boot run --save-state-trace b.state (e.g. with --patch-byte)"] --> dv
  dv -->|Identical / LengthDiffers| done["verdict"]
  dv -->|"Differs at step N, field Pc or Hash"| win["re-capture both with a full-state window around N"]
  dv -->|CorruptTrace| bad["exit 3, no verdict"]
  win --> zm["cellgov diff zoom a.zoom b.zoom N"]
  zm --> rd["RegDiff list: the fingerprint fields that differ"]
```

## RPCS3 bridge

`bridges/rpcs3-patch/0001-cellgov-checkpoint-dump.patch` adds an
opt-in dump hook to RPCS3's `_sys_process_exit` syscall. With
`CELLGOV_DUMP_PATH` and `CELLGOV_DUMP_REGIONS` set, RPCS3 writes
the configured guest memory regions (parsed as `addr:size` hex
pairs, appended contiguously in declaration order) to a binary
file on process exit. `bridges/rpcs3_to_observation` converts that
dump plus a shared region manifest into the same `Observation` JSON
`cellgov diff observations` reads.

The user builds the patched RPCS3 binary; the CellGov library has no
Cargo or runtime dependency on RPCS3, and the bridge is a
verification-time tool. See
a cell's `REPRODUCTION.md` under
`tests/fixtures/<content-id>/cross_runner/fw-<ver>/<game-ver>/` for the
build commands and the build-config workarounds.

## Oracle-mode config contract

An RPCS3 observation serves as an oracle only when RPCS3 is
configured for deterministic PPU/SPU behavior and no RSX/audio
output: `Video.Renderer = "Null"`, `Audio.Renderer = "Null"`,
`Core.PPU Decoder = Recompiler (LLVM)`, and `Core.SPU Decoder =
Recompiler (LLVM)`. The canonical YAML for these four fields is
embedded in `bridges/rpcs3_to_observation/`; the adapter hashes it
(FNV-1a) at build time and requires a matching `--config-hash` on
every invocation, so a dump produced under a different RPCS3 config
is rejected at adapter entry instead of feeding a wrong-config
observation into the comparator.
`rpcs3_to_observation --print-expected-config-hash` prints the
expected hash for scripting.
