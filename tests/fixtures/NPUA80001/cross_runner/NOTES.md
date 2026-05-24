---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 11271
convergence: No (outcome: ProcessExit vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at the manifest's `process-exit`
checkpoint: outcomes are distinct.

CellGov terminates with `ProcessExit` at step 11,271 (+15
from the Phase 36.7 anchor of 11,256). The +15 step advance
comes from Phase 37's 675 modeling: `sys_rsx_device_map`
now returns CELL_OK with a non-zero device address, so
libgcm proceeds slightly further into `cellGcmInit` before
the next gap surfaces. flOw still ends up in its abort path
because libgcm's later setup still hits unmodeled state
(the unbacked mmapper handout window is one such gap; see
WipEout NOTES for the structural blocker the next phase
addresses). flOw's CRT0 invokes
`sys_process_exit(1)` cleanly on the abort path.

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
