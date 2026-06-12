---
content_id: NPUA80068
title: Super Stardust HD
year: 2007
developer: Housemarque
engine: Housemarque proprietary
distribution: PSN HDD
checkpoint: FirstRsxWrite (requested; not reached)
steps: 390433
convergence: No (outcome: Timeout vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at FirstRsxWrite: CellGov
times out at `MaxSteps` (390,433 under default budget) before
reaching the first RSX put-pointer write; RPCS3 completes
the checkpoint. Byte parity undefined until convergence.

Phase 39's BE-FIFO decode + IO->EA translation correctness
fixes plus the `sys_rsx_context_attribute` FIFO_SETUP arm
advanced SSHD past its prior `14,341,833 / Fault` at the
RSX device-enumeration codepath; the Fault no longer fires.
SSHD now caps at the same `MaxSteps` budget flOw hits at
the shared firmware libgcm spin-poll on `dma.ref` at
`0x7a08`. Under `--budget 1` SSHD runs through 50M+ steps
without re-hitting any Fault, confirming the prior fault
path is downstream of one of the corrected boot stages.
One residual host invariant break at the new anchor
(390,433 steps): an unclaimed syscall 465 returning
CELL_ENOSYS.

The prior `14,341,833 / Fault` with three honest residual
`host_invariant_breaks` (two
`dispatch.ppu_thread_create_unmodeled_flags` for the
`flags=0x10000` convergent gap, one no-op-with-trace) is
preserved as a documented downstream code path that does
not re-fire under the new trajectory.

RPCS3 corpus state (Stage E):
  outcome: Completed
  step: (not recorded in observation)
  checkpoint: FirstRsxWrite (manifest)
  reached: yes -- RPCS3 boots SSHD end-to-end through the
           operator-declared FirstRsxWrite point.

## Next step

SSHD's fault is orthogonal to the 670 / 675 / 672 modeling
that Phase 37 landed (verified: SSHD's anchor is unchanged
at 14,341,833 / Fault / breaks=3 across Phase 36.7 and
Phase 37). The downstream RSX-init divergence sits past the
unbacked-mmapper blocker WipEout currently hits at step
43,066; a successor phase backs the mmapper handout window
and re-measures SSHD to confirm which RSX-init step is the
next honest fault. The 0x10000 thread-flag log can downgrade
from invariant-break to a one-line note in the same effort
if a lie-vs-gap classifier emerges; until then the count of
3 reflects honest "unmodeled" reports against RPCS3-faithful
behavior plus the unmodeled-no-op handler logs the boot
triggers.
