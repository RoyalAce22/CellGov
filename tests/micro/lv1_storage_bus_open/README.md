# lv1_storage_bus_open

A read-only capture of lv1's storage bus object on the reference console
at firmware 4.93, after the boot to the XMB. The 4.93 kernel opens the
storage bus with hypercall 170 (`open_device`) at boot. The capture shows
that lv1 holds the storage bus open for the game OS partition. CellGov's
hypervisor answers the kernel's open through
`cellgov_ps3_abi::lv1::device_bus`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1` only, through the
  runner's console queue, on 2026-11-17. The command reads webMAN's
  `peek.lv1` pages. The capture wrote nothing to lv1 or lv2, used no
  poke, sent no hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
  An earlier run of the same reads, before the files were written, read
  the same values.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the call. The storage bus search sends
hypercall 170 with r3 = the bus id (4), r4 = -1 and r5 = 0. It goes on
only when r3 is 0
[OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae118].

## Where lv1 keeps the open

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path:

- At 4.93 the hypercall vector loads the TOC address 0x35A038 into r2
  [OFW-4.93 lv1 a:0x0fa4-0x0fac].
- The open function sends bus 4 to the lookup
  [OFW-4.93 lv1 a:0x237714-0x237718]. For bus 4, the lookup reads the
  storage bus object from the global that the TOC slot at r2 - 26656
  locates [OFW-4.93 lv1 a:0x2365E0-0x2365E8]. The slot holds 0x368388
  [OFW-4.93 lv1 a:0x353818-0x35381C]. For device -1, the lookup returns
  the object itself [OFW-4.93 lv1 a:0x2365F0-0x2365FC].
- The open function calls the method at +24 of the object's method
  table, with the caller's partition and the call's r5. It returns that
  method's r3 zero-extended [OFW-4.93 lv1 a:0x23760C-0x237630].
- The object's constructor stores the method table at +0
  [OFW-4.93 lv1 a:0x259298] [OFW-4.93 lv1 a:0x2592AC]. The TOC slot that
  it reads holds 0x35DD30 [OFW-4.93 lv1 a:0x354670-0x354674]. It sets
  the byte at +3496 to 1 [OFW-4.93 lv1 a:0x25929C-0x2592A4].
- The method at +24 is the open method
  [OFW-4.93 lv1 a:0x35DD48-0x35DD4C] [OFW-4.93 lv1 a:0x335568-0x33556C].
  It calls the partition check [OFW-4.93 lv1 a:0x259098-0x2590A0] and
  returns a nonzero status of the check
  [OFW-4.93 lv1 a:0x2590A4-0x2590C4].
- The partition check reads lpar 1's `ss.laid` node for the partition
  and tests the byte at +3496 [OFW-4.93 lv1 a:0x2545B8-0x2545F4]. Under
  the lock at +2536 it tests two 64-byte slots, at +2544 and then at
  +2608 [OFW-4.93 lv1 a:0x254604-0x254660]. A free slot ends the test,
  so a free first slot is taken before the second slot is read
  [OFW-4.93 lv1 a:0x25462C-0x254638]. A used slot that holds the
  caller's partition gives -9 [OFW-4.93 lv1 a:0x254734], and two used
  slots that hold other partitions give -2 [OFW-4.93 lv1 a:0x254658].
  Otherwise it takes the free slot. It keeps the partition at +0, the
  call's r5 at +8, sets the byte at +16 and returns 0
  [OFW-4.93 lv1 a:0x254710-0x25472C].
- After a 0, the open method allocates a record, keeps the partition at
  +40 of the record and links the record into the list at +3552. The
  record's +32 holds the list. The list's +8 holds its last record
  [OFW-4.93 lv1 a:0x2590E8-0x2590F8] [OFW-4.93 lv1 a:0x259204-0x259258].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| storage bus object pointer | 0x368388 | 0x7DDC0 |
| object + 0, method table | 0x7DDC0 | 0x35DD30, the constructor's table |
| object + 3496, gate byte | 0x7EB68 | 0x01 |
| slot 0 + 0, partition | 0x7E7B0 | 1 |
| slot 0 + 8, r5 | 0x7E7B8 | 0 |
| slot 0 + 16, used | 0x7E7C0 | 0x01 |
| slot 1 + 0, partition | 0x7E7F0 | 2 |
| slot 1 + 8, r5 | 0x7E7F8 | 0 |
| slot 1 + 16, used | 0x7E800 | 0x01 |
| list + 8, last record | 0x7EBA8 | 0x38B590 |
| record + 32, list | 0x38B5B0 | 0x7EBA0, the object's list |
| record + 40, partition | 0x38B5B8 | 2 |

## The values

- The object holds the constructor's method table, so it is the object
  whose open path the fix reads.
- The gate byte is 1, the value that the constructor sets.
- Slot 1 holds partition 2, the game OS partition, with r5 = 0. The
  check writes a slot only on the path that returns 0. So the checks
  passed for the game OS partition's open, and that open sent r5 = 0,
  as the kernel's call does.
- Slot 0 holds partition 1. So partition 1 opened the storage bus first,
  and the game OS partition took the free slot 1.
- The last record of the list is in the object's list and holds
  partition 2. So the allocation after the check succeeded, and the
  open method returned 0.

## Files

| File | Rows | Holds |
|---|---|---|
| `walk.tsv` | 2 | the doublewords at the storage bus object pointer and at the object's +0 |
| `gate.tsv` | 1 | the byte at the object's +3496 |
| `slots.tsv` | 4 | the partition and r5 doublewords of slots 0 and 1 |
| `slot_used.tsv` | 2 | the used bytes of slots 0 and 1 |
| `record.tsv` | 3 | the list's last record, and that record's +32 and +40 |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x368388,0x7DDC0 --out walk.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x7EB68 --out gate.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7E7B0,0x7E7B8,0x7E7F0,0x7E7F8 --out slots.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x7E7C0,0x7E800 --out slot_used.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EBA8,0x38B5B0,0x38B5B8 --out record.tsv
```
