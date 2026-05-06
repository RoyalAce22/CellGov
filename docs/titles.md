# Tested Titles

Boot-frontier compatibility matrix. Each row is a title CellGov boots
to a checkpoint whose observation is byte-equivalent (modulo
classified non-semantic divergences) to an RPCS3 baseline at the
same checkpoint.

This is not gameplay compatibility. See
[architecture.md](architecture.md) for what CellGov does and does
not model.

## Reading the table

Terminology used in the Cross-runner column comes from
[concepts.md](concepts.md). In brief:

- `equivalent (N bytes non-semantic)` means the raw byte-level
  comparison found N differing bytes, every one of which has been
  classified as not affecting program behaviour (HLE OPD layout,
  SELF reconstruction metadata, etc.).
- `DIVERGE` (without the `equivalent` qualifier) means the
  divergence is either unclassified or includes at least one
  semantic difference. No title currently in the matrix sits in
  that state.

Column definitions:

- **Steps**: unit yields at the default per-step budget (256
  instructions). Use `--budget 1` for single-instruction stepping.
- **Insns**: rough instruction count to checkpoint (`steps * budget`).
- **Checkpoint**: the deterministic boot stopping point.
- **Cross-runner**: classified verdict against the RPCS3 baseline.

## Matrix

| Serial | Title | Year | Engine | Format | Checkpoint | Steps | Insns | Cross-runner |
|--------|-------|------|--------|--------|------------|------:|------:|--------------|
| NPUA80001 | flOw | 2007 | thatgamecompany | PSN HDD | FAULT (NULL bcctr in m_InitEntityHierarchy) | 100,047 | ~26M | outcome mismatch (FAULT vs Completed; needs shared-checkpoint flag) |
| NPUA80068 | Super Stardust HD | 2007 | Housemarque | PSN HDD | FirstRsxWrite | 14,352,589 | ~3.7B | non-semantic (ELF e_ehsize at 0x35) |
| BCES00664 | WipEout HD Fury | 2009 | Sony Liverpool | Disc ISO | FirstRsxWrite | 45,697 | ~12M | non-semantic (ELF e_version at 0x17) |

flOw's frontier moved from step 85,291 to 100,047 (+14,756 steps)
once the worker-thread callback-dispatch primitive landed and
`cellSaveDataAutoLoad` ran the title's `funcStat` callback for
real (the no-save-data path). The title's funcStat dump-formatter
ran to completion, the title accepted `CELL_SAVEDATA_ERROR_NODATA`
(`0x8002b40b`) as the call return, `thr_auto_load()` finished, and
the title entered `CFlOwApplication::m_InitEntityHierarchy`. The
new fault is the same NULL-bcctr-into-vtable shape (LR=0x000923e8)
but in entity-init code rather than save-data return-path code,
and it surfaces inside a child PPU thread (r1 in the child-stack
region, r2=0). The named successor driver is whichever uninitialized
vtable slot the entity-init code dereferences -- most likely a
sysmodule that `cellSysmoduleLoadModule` was supposed to populate.

SSHD's anchor at 14,352,589 and WipEout HD's at 45,697 are bit-
identical across the current correctness surface. WipEout's earlier
MaxSteps anchor is gone -- sync-primitive correctness lets its boot
proceed past the lwmutex / event_flag contention loops it
previously spun in.

Cross-runner refresh: post-Phase-29 the three reports were
regenerated against the existing RPCS3 baselines. SSHD and
WipEout reach FirstRsxWrite cleanly and diverge only on
non-semantic ELF-header reconstruction bytes (`e_ehsize` at 0x35
for SSHD, `e_version` at 0x17 for WipEout); the same class
flagged in earlier reports. flOw's CellGov-side now terminates in
a fault before the shared first-sys_tty_write checkpoint RPCS3
captures at, producing an outcome-class mismatch the byte-level
comparator stops on; a CellGov-side "stop at Nth sys_tty_write"
flag (mirroring `CELLGOV_DUMP_TTY_NTH`) is the prerequisite for
returning flOw to byte-level comparable. Per-title compare reports
live at `tests/fixtures/<serial>_cross_runner/compare_report.txt`.
