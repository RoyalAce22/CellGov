---
content_id: NPUA80068
title: Super Stardust HD
year: 2007
developer: Housemarque
engine: Housemarque proprietary
distribution: PSN HDD
checkpoint: FirstRsxWrite
steps: 14341833
convergence: No (outcome: Fault vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at FirstRsxWrite: CellGov faults
at step 14,341,833; RPCS3 completes the checkpoint. Byte
parity undefined until convergence. Re-anchored under the
complete firmware-set boot: the prior fault at step
14,342,058 was measured under
contamination from `dispatch.unresolved_import` (the
`cellVideoOutGetScreenSize` NID required by SSHD was the
lone outlier the closure walk surfaced -- exported by
`libsysutil_avconf_ext`, now in MIN_VIABLE_PRX_STEMS).

Post-re-anchor: zero unresolved-import breaks; closure walk
closes. The 3 remaining host invariant breaks are all honest:
two `dispatch.ppu_thread_create_unmodeled_flags` firings for
`flags=0x10000` (a convergent honest gap -- RPCS3's
`_sys_ppu_thread_create` only consults `flags & 3` per
`tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_ppu_thread.cpp:492`,
silently ignoring bit `0x10000`; CellGov matches), plus one
log from one of the unmodeled-no-op handlers (memory_free,
spu_initialize, RSX free, event-port-connect-local ENOSYS, or
similar) the boot exercises. The downstream fault is not
derived from these honest gaps; it is a separate RSX-init
divergence the RSX-init progression work owns.

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
