---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 11256
convergence: No (outcome: ProcessExit vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at the manifest's `process-exit`
checkpoint: outcomes are distinct. Re-anchored under the
complete firmware-set boot: the prior anchor measured an
under-loaded early-exit on failed module load (see
`docs/dev/bug_investigations/firmware_set_closure.md`).

CellGov terminates with `ProcessExit` at step 11,256 because
the firmware-set boot now loads the full 15-stem set
(MIN_VIABLE_PRX_STEMS), so the early `cellSysmoduleLoadModule`
calls succeed against `libsysmodule`/`libgcm_sys`/etc. and the
title proceeds past the previous EINVAL contamination. Boot
then runs through libgcm's `cellGcmInit()`, which calls
`sys_rsx_device_map` (LV2 syscall 675). CellGov does not model
sys_rsx_device_map; the unsupported-syscall path returns
`CELL_ENOSYS` (the honest "not implemented" errno, replacing
the prior fake-CELL_OK lie). libgcm honestly reports
"cellGcmInit() failed", flOw's CRT0 enters its abort path
("Waiting for the SPU thread group to be terminated...",
`sys_spu_thread_group_join` fails with CELL_ESRCH because no
group was created, abort() is called), and the title invokes
`sys_process_exit(1)` cleanly. 43 host invariant breaks during
this run are honest ENOSYS / no-op-with-trace returns for the
unmodeled syscalls libgcm and the abort path exercise -- mostly
`dispatch.unsupported_stub` for RSX syscalls, plus one
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

The RSX-init progression work closes this divergence: model
`sys_rsx_device_map` (LV2 syscall 675) and the sister RSX
syscalls (673 / 674 / 676 / 677) so libgcm's `cellGcmInit`
succeeds and flOw proceeds past the abort path. With
cellGcmInit succeeding, flOw's gameplay-loop boot path runs
the same instruction trace RPCS3 records; the two runners
can then converge at the manifest's ProcessExit checkpoint.
