---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit (requested; not reached)
steps: 390625
convergence: No (outcome: Timeout vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at the manifest's `process-exit`
checkpoint: outcomes are distinct.

CellGov advances 390,625 steps under the default budget (256)
and times out at `MaxSteps` -- the requested `process-exit`
checkpoint is no longer reached because phase 39's BE-FIFO
decode + IO->EA translation correctness fixes plus the
`sys_rsx_context_attribute` FIFO_SETUP arm let `cellGcmInit`
complete far enough to enter the firmware libgcm spin-poll
on `dma.ref` at `0x7a08`. The title is now waiting for an
RSX completion token that CellGov does not yet publish; an
honest FIFO-completion consumer (designed during phase 39,
landing alongside phase 40) is the next clearing step.

The prior `11,271 / ProcessExit` line (CRT0 abort after
`cellGcmInit() failed`) is preserved as a documented
downstream-of-`0x7a08` code path that no longer fires
under the new boot trajectory.

The 42 host invariant breaks during this run are honest:
ENOSYS / no-op-with-trace returns for the unmodeled syscalls
libgcm and the abort path exercise -- `dispatch.unsupported_stub`
for the RSX syscalls Phase 37 did not model, the
`dispatch.mmapper_map_shared_memory_unbacked` entries for
the new honest mmapper gap, and one
`dispatch.memory_free_noop` from the boot's first call to
sys_memory_free against the bump allocator. No contaminating
falsehoods.

RPCS3 corpus state (Stage E):
  outcome: Completed
  step: (not recorded in observation)
  checkpoint: ProcessExit (manifest)
  reached: yes -- RPCS3 boots flOw end-to-end through the
           gameplay loop and the operator-declared
           ProcessExit point.

The byte walk does not run; byte parity is undefined while
the outcomes differ.

## Next step

A successor phase backs the mmapper handout window so
libgcm's `_cellGcmInitBody` succeeds end-to-end, the abort
path no longer fires, and flOw proceeds to its gameplay loop.
The two runners can then converge at the manifest's
ProcessExit checkpoint.
