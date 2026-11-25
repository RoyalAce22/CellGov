# lv1_device_bus_open

A read-only capture of device 0 of bus 1 in lv1's system bus object, on
the reference console at firmware 4.93, after the boot to the XMB. The
4.93 kernel opens that device with hypercall 170 (`open_device`) at
boot. The capture shows that lv1 holds the device open for the game OS
partition. CellGov's hypervisor answers the kernel's open through
`cellgov_ps3_abi::lv1::device_bus`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`. `cellgov ps3
  status` passed the profile before the reads.
- The capture used `cellgov ps3 peek --space lv1` only, through the
  runner's console queue, on 2026-11-14. The command reads webMAN's
  `peek.lv1` pages. The capture wrote nothing to lv1 or lv2, used no
  poke, sent no hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
  A second run of each command, before the files were written, read the
  same values.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: both rows
  are lv2 ranges. No address is MMIO.
- No console-identifying value is in this directory.

## Where lv1 keeps the device

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path:

- At 4.93 the hypercall vector loads the TOC address 0x35A038 into r2
  [OFW-4.93 lv1 a:0x0fa4-0x0fac].
- The getter of the system bus object reads the doubleword that the TOC
  slot at r2 - 27872 locates, and returns the doubleword there
  [OFW-4.93 lv1 a:0x22B6BC-0x22B6C4]. The slot holds 0x3682C0
  [OFW-4.93 lv1 a:0x353358-0x35335C].
- The lookup for bus 1 gets the object
  [OFW-4.93 lv1 a:0x235D88-0x235DA0] and reads device 0 from slot 0
  [OFW-4.93 lv1 a:0x235DC8-0x235DE8], the doubleword at object + 32
  [OFW-4.93 lv1 a:0x22B838-0x22B844]. lv1's init fills that slot
  [OFW-4.93 lv1 a:0x3A9924-0x3A9948].
- The device's constructor stores its method table at +0
  [OFW-4.93 lv1 a:0x22DDC0-0x22DDD4]. The TOC slot that it reads holds
  0x35C878 [OFW-4.93 lv1 a:0x353480-0x353484].
- The device's open entry holds the lock word at +104
  [OFW-4.93 lv1 a:0x228078-0x228094]. When the byte at +108 is 0, it
  stores its r4 at +112 as a doubleword, sets the byte at +108 to 1 and
  returns 0 [OFW-4.93 lv1 a:0x2280A0-0x2280BC].
- The handler gives the open function the caller's partition in r3
  [OFW-4.93 lv1 a:0x305AD0-0x305AEC]. The open function keeps it
  [OFW-4.93 lv1 a:0x2375DC] and gives it to the open entry in r4
  [OFW-4.93 lv1 a:0x2376A8].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| system bus object pointer | 0x3682C0 | 0x61D710 |
| object + 32, slot 0 | 0x61D730 | 0x61D900, the device |
| device + 0, method table | 0x61D900 | 0x35C878, the constructor's table |
| device + 108, open flag | 0x61D96C | 0x01 |
| device + 112, partition | 0x61D970 | 2 |

## The values

- The device's method table is the table that the constructor stores, so
  slot 0 holds the device whose open path the fix reads.
- The open flag is 1. lv1 sets it only on the path that returns 0. When
  the hook after the open fails, the open function closes the device
  again [OFW-4.93 lv1 a:0x2377B4-0x2377D8]. So the device's last open
  returned 0, and no close followed it.
- The partition is 2, the game OS partition, which CellGov's hypervisor
  gives the kernel as its partition id
  (`cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS`).

## Files

| File | Rows | Holds |
|---|---|---|
| `walk.tsv` | 3 | the doublewords at the system bus object pointer, at slot 0 and at the device's +0 |
| `open_flag.tsv` | 1 | the byte at the device's +108 |
| `partition.tsv` | 1 | the doubleword at the device's +112 |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x3682C0,0x61D730,0x61D900 --out walk.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x61D96C --out open_flag.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D970 --out partition.tsv
```
