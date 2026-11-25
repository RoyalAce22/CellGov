# lv1_storage_device_open

A read-only capture of lv1's storage device objects on the reference
console at firmware 4.93, after the boot to the XMB. The 4.93 kernel
opens each storage device that lv1 reports with hypercall 170
(`open_device`). The capture shows that lv1 holds each of the five
devices of the bus table open for the game OS partition. CellGov's
hypervisor answers the kernel's opens through
`cellgov_ps3_abi::lv1::device_bus`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1` only, through the
  runner's console queue, on 2026-11-19. The command reads webMAN's
  `peek.lv1` pages. The capture wrote nothing to lv1 or lv2, used no
  poke, sent no hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
- Every address is lv1 heap or lv1 data. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No address is MMIO.
- The device object addresses come from `devices.tsv` of the capture
  `lv1_storage_registration`: the doublewords at +3816 + 8 * id of the
  storage bus object, for ids 0 to 4.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the call. The kernel's device object sends
hypercall 170 with r3 = its +144, r4 = its +152 and
r5 = 0x8000000000000000
[OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac120]. The
object's constructor copies +144 and +152 from the bus id and the
device id of the kernel's device record
[OFW-4.93 lv2_kernel a:0x80000000000a98c8-0x80000000000a98e8].

## Where lv1 keeps the open

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. D is the device object.

- For a device id up to 6, the lookup for bus 4 returns D, the
  doubleword at +3816 + 8 * id of the storage bus object
  [OFW-4.93 lv1 a:0x236604-0x23661C].
- The open function calls the method at +24 of D's method table, with
  D, the caller's partition and the call's r5. It returns that method's
  r3 zero-extended [OFW-4.93 lv1 a:0x23760C-0x237630].
- The entry at +24 of each of the five method tables holds the
  descriptor 0x334BF0 [OFW-4.93 lv1 a:0x35E530-0x35E534]
  [OFW-4.93 lv1 a:0x35E700-0x35E704] [OFW-4.93 lv1 a:0x35E388-0x35E38C]
  [OFW-4.93 lv1 a:0x35E1E0-0x35E1E4] [OFW-4.93 lv1 a:0x35E010-0x35E014].
  The descriptor's entry is the partition check 0x2545A4
  [OFW-4.93 lv1 a:0x334BF0-0x334BF4], which the storage bus's open
  method also calls.
- The check reads lpar 1's `ss.laid` node for the partition. It then
  tests D's byte at +3496 [OFW-4.93 lv1 a:0x2545B8-0x2545E4].
- It tests the doubleword at +2352 of the object at D + 144. When that
  doubleword is 0, it calls the method at +296 of D's table
  [OFW-4.93 lv1 a:0x2545E8-0x2545F4] [OFW-4.93 lv1 a:0x254694-0x2546B4].
  When both are 0, the partition's authority id decides
  [OFW-4.93 lv1 a:0x2546B8-0x2546E8].
- It keeps the partition at +112 and sets the byte at +108
  [OFW-4.93 lv1 a:0x2545F8-0x254600].
- Under the lock at +2536 it tests two 64-byte slots, at +2544 and then
  at +2608. A free slot ends the test. A used slot that holds the
  caller's partition gives -9. Two used slots that hold other
  partitions give -2 [OFW-4.93 lv1 a:0x254604-0x254660]
  [OFW-4.93 lv1 a:0x254734].
- Otherwise it takes the free slot. It keeps the partition at +0, the
  call's r5 at +8, sets the byte at +16 and returns 0
  [OFW-4.93 lv1 a:0x254710-0x25472C].

## The values

| Device | Type | D | Method table | +2352 of D + 144 | Slot 0 | Slot 1 |
|---|---|---|---|---|---|---|
| 0 | 0xFF | 0x628010 | 0x35E518 | 0 | partition 2, r5 0x8000000000000000 | free |
| 1 | 0xFE | 0x6299D0 | 0x35E6E8 | 2 | partition 1, r5 0 | partition 2, r5 0x8000000000000000 |
| 2 | 5 | 0x660010 | 0x35E370 | 0 | partition 2, r5 0x8000000000000000 | free |
| 3 | 0 | 0x6655A0 | 0x35E1C8 | 2 | partition 2, r5 0x8000000000000000 | free |
| 4 | 14 | 0x368398 | 0x35DFF8 | 2 | partition 1, r5 0 | partition 2, r5 0x8000000000000000 |

- Each device holds the method table that the registration capture
  reads at its +0.
- The +144 of each device points at its own +152.
- Each device's byte at +3496 is 1. Its byte at +108 is 1, and its +112
  holds partition 2.
- Devices 0 and 2 hold 0 at +2352 of D + 144. Their types are not 0,
  and the registration writes the type from D's method at +296.
- The check writes a slot only on the path that returns 0. Each device
  has one slot that holds partition 2, the game OS partition, with the
  kernel's r5. So the checks passed for each of the game OS partition's
  opens.
- For each device, the first slot that is free or holds partition 2
  holds partition 2. So a second open of the game OS partition gets -9.

## Files

| File | Rows | Holds |
|---|---|---|
| `walk.tsv` | 10 | the doublewords at +0 and +144 of each device |
| `gate.tsv` | 5 | the byte at +3496 of each device |
| `sub.tsv` | 5 | the doubleword at +2352 of the object at +144 of each device |
| `slots.tsv` | 20 | the partition and r5 doublewords of slots 0 and 1 of each device |
| `slot_used.tsv` | 10 | the used bytes of slots 0 and 1 of each device |
| `open_flag.tsv` | 5 | the byte at +108 of each device |
| `owner.tsv` | 5 | the doubleword at +112 of each device |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x628010,0x6299D0,0x660010,0x6655A0,0x368398,0x6280A0,0x629A60,0x6600A0,0x665630,0x368428 --out walk.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x628DB8,0x62A778,0x660DB8,0x666348,0x369140 --out gate.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x6289D8,0x62A398,0x6609D8,0x665F68,0x368D60 --out sub.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x628A00,0x628A08,0x628A40,0x628A48,0x62A3C0,0x62A3C8,0x62A400,0x62A408,0x660A00,0x660A08,0x660A40,0x660A48,0x665F90,0x665F98,0x665FD0,0x665FD8,0x368D88,0x368D90,0x368DC8,0x368DD0 --out slots.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x628A10,0x628A50,0x62A3D0,0x62A410,0x660A10,0x660A50,0x665FA0,0x665FE0,0x368D98,0x368DD8 --out slot_used.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x62807C,0x629A3C,0x66007C,0x66560C,0x368404 --out open_flag.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x628080,0x629A40,0x660080,0x665610,0x368408 --out owner.tsv
```
