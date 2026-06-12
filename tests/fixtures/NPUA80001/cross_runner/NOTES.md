---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 11224
convergence: No (outcome: ProcessExit vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at the manifest's `process-exit`
checkpoint: outcomes are distinct.

CellGov reaches `process-exit` at step 11,299 (cumulative
2,892,544 instructions at the default budget of 256 per step).
The boot trajectory is an EARLY ABORT: the title calls
`sys_process_exit(CELL_EABORT)` at step 5910, then runs a 5,389-
step atexit / cleanup sequence (which generates 43 host-invariant
breaks: 1 SPU initialize, 1 PPU thread create with unmodeled
flag, 2 RSX `cellGcmResetFlipStatus`, 39 `_sys_prx_stop_module`
across 13 module IDs x 3 stacks), then calls
`sys_process_exit(1)` at step 11,299. The runtime detects
`NoRunnableUnit` and reports `outcome=ProcessExit`.

Why the abort: zero host-invariant breaks fire in `[0, 5910)`,
so the title's pre-abort fatal condition came back through a
normal syscall return (a wrong-value path) or a guest-side read
of zeroed engine state. Named candidates: the 528 silently-no-
op'd NV4097 methods in the bring-up FIFO; the absent SPURS SPU
thread spawn (BENCH_SPU_THREAD_INIT_WITNESS: count=0); the
unwired `sys_rsx_context_attribute(package_id=0x10A)` arm
returning CELL_EINVAL. Pinning the specific trigger is the
entry point for a future ISA-coverage / SPURS-bringup phase.

The 43 host-invariant breaks are all expected-benign per
`docs/dev/bug_investigations/flow_post_40f_libgcm_ref_addr_spin.md`
("Pre-close verification" section): the SPU + PPU init / thread
warnings are pre-existing; the RSX attribute and PRX stop_module
arms are honest not-implemented ENOSYS/EINVAL returns.

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

The next-phase entry point per the closed REF_ADDR finding is
the small load-bearing subset of NV4097 / SPURS / RSX-attribute
methods the pre-abort init path actually depends on -- not the
whole NV4097 table. Probe each pre-step-5910 title-side read
against the zero-state surface, identify the FIRST unexpected
value, trace backward to the producing method. Implement only
the methods the abort actually reads. After that load-bearing
subset lands, flOw's pre-abort init can proceed and the two
runners can converge at the manifest's ProcessExit checkpoint.
