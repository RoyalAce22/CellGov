---
content_id: NPUA80001
title: flOw
year: 2007
developer: thatgamecompany
engine: PhyreEngine
distribution: PSN HDD
checkpoint: ProcessExit
steps: 117187
convergence: No (outcome: Timeout vs Completed)
byte_parity: --
---

flOw does not converge with RPCS3: CellGov terminates with
outcome `Timeout` (MaxSteps at 117,187) against the captured RPCS3
`Completed` at first `sys_tty_write`. The byte walk does not run;
byte parity is undefined.

## Next step

Extend CellGov with a "stop at Nth `sys_tty_write`" checkpoint
mirroring `CELLGOV_DUMP_TTY_NTH` in the patched RPCS3, capture a
refreshed observation at that shared checkpoint, then regenerate
without `--allow-divergence`.
