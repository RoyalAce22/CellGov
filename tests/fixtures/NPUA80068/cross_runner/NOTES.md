---
content_id: NPUA80068
title: Super Stardust HD
year: 2007
developer: Housemarque
engine: Housemarque proprietary
distribution: PSN HDD
checkpoint: FirstRsxWrite
steps: 14342058
convergence: No (outcome: Fault vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at FirstRsxWrite: CellGov faults
at step 14,342,058 on an unresolved-import trampoline call
for `cellSysmoduleInitialize` (NID `0x63ff6ff9`); RPCS3
completes the checkpoint. Byte parity undefined until
convergence.

RPCS3-side observation in this directory is from an earlier
capture and may need re-running against the current build of
`tools/rpcs3/rpcs3.exe` -- see REPRODUCTION.md for the
capture commands.

## Next step

Resolve the unresolved-import fault driver -- either map the
NID to a direct LV2 syscall handler or include `cellSysmodule`
in the loaded firmware PRX set -- then regenerate the fixture.
