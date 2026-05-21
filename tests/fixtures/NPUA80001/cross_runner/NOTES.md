---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 9048
convergence: No (outcome: ProcessExit vs Completed)
byte_parity: --
---

Does not converge with RPCS3 at the manifest's `process-exit`
checkpoint: outcomes are distinct. CellGov terminates with
`ProcessExit` at step 9,048 because an unresolved-import
trampoline call for `cellSysmoduleLoadModule` (NID
`0x32267a31`) returns CELL_EINVAL, which the title's CRT0
routes into `sys_process_exit`. The RPCS3-side capture
terminates with outcome `Completed` -- a different
terminal state, captured at the operator-declared
checkpoint, not at a guest-side `sys_process_exit`. The
title is playable end-to-end on RPCS3; it does not exit
early via `sys_process_exit` there.

The byte walk does not run; byte parity is undefined.

## Next step

Resolve the unresolved-import driver -- either route
`cellSysmoduleLoadModule` to a direct LV2 syscall handler or
include `cellSysmodule` in the loaded firmware PRX set -- so
CellGov can advance past step 9,048. Then either pick a
shared PC-based checkpoint reachable on both runners or
re-capture RPCS3 at the matching ProcessExit point.
