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

| Serial    | Title             | Year | Engine          | Format   | Checkpoint                                                        |      Steps | Insns | Cross-runner                                                           |
| --------- | ----------------- | ---- | --------------- | -------- | ----------------------------------------------------------------- | ---------: | ----: | ---------------------------------------------------------------------- |
| NPUA80001 | flOw              | 2007 | thatgamecompany | PSN HDD  | MaxSteps (SPURS handler-thread parked on sys_event_queue_receive) |    195,312 |  ~50M | outcome mismatch (MaxSteps vs Completed; needs shared-checkpoint flag) |
| NPUA80068 | Super Stardust HD | 2007 | Housemarque     | PSN HDD  | FirstRsxWrite                                                     | 14,352,589 | ~3.7B | non-semantic (ELF e_ehsize at 0x35)                                    |
| BCES00664 | WipEout HD Fury   | 2009 | Sony Liverpool  | Disc ISO | FirstRsxWrite                                                     |     45,697 |  ~12M | non-semantic (ELF e_version at 0x17)                                   |

flOw's frontier moved from step 86,527 to 195,312 (+108,785 steps)
once the cellFs real-disk VFS landed and the kernel-fd-range fix
(`LV2_FS_OBJECT_ID_BASE = 3`, matching real PS3's `lv2_fs_object::id_base`)
let the inline `cellFsRead` wrapper see the fds in its
expected `[3, 255)` range. Resource enumeration (every file under
`/app_home/Data/**`) succeeds, the entity tree is fully populated,
and the `m_InitEntityHierarchy` NULL-bcctr chain clears. Boot now
runs through video config, GCM init, SPURS PPU-surface
initialization, and the title's pthread coordination barrier
where it parks. The primary thread spins on the sysPrxForUser
`sys_lwmutex_lock` / `_unlock` / `sys_ppu_thread_get_id` HLE-import
trio while a sibling event-handler thread (`entry=0x9b0a0`) parks
on `sys_event_queue_receive` waiting for SPURS workload-completion
events. The named successor driver is the SPURS handler-thread
event producer: cellSpurs HLE maintains the user-space SPURS
struct but never starts an SPU thread group, so no completion
events ever fire.

SSHD's anchor at 14,352,589 and WipEout HD's at 45,697 are bit-
identical across the current correctness surface. WipEout's earlier
MaxSteps anchor is gone -- sync-primitive correctness lets its boot
proceed past the lwmutex / event_flag contention loops it
previously spun in.
