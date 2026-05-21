---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 9048
convergence: Yes
byte_parity: 556 non-semantic
---

Converges with RPCS3 at ProcessExit (CellGov step 9,048).
CellGov exits via an unresolved-import trampoline call for
`cellSysmoduleLoadModule` (NID `0x32267a31`): the trampoline
issues `Lv2Request::UnresolvedImport`, which the dispatcher
turns into CELL_EINVAL, and the title's CRT0 routes that
into `sys_process_exit`. RPCS3 reaches its own ProcessExit
through a different path; both observations terminate with
outcome `ProcessExit`, satisfying the manifest's
`process-exit` checkpoint.

Byte parity: 556 bytes diverge, all classified as
`HleOpdSlot` (the secondary OPD table at `0x82eea8` populated
differently on each runner). No unclassified residual.

RPCS3-side observation in this directory is from an earlier
capture and may need re-running against the current build of
`tools/rpcs3/rpcs3.exe` -- see REPRODUCTION.md for the
capture commands.
