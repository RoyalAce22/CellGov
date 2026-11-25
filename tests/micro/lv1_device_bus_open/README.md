# lv1_device_bus_open

A read-only capture of devices 0, 1 and 2 of bus 1 in lv1's system bus
object, on the reference console at firmware 4.93, after the boot to
the XMB. The 4.93 kernel opens the three devices with hypercall 170
(`open_device`) at boot. The capture shows that lv1 holds each device
open for the game OS partition. CellGov's hypervisor answers the
kernel's opens through `cellgov_ps3_abi::lv1::device_bus`.

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

## Device 1

The 4.93 kernel opens device 1 of bus 1 with r3 = 1, r4 = 1 and r5 = 0
[OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718]. lv1's
path for that device:

- The lookup for bus 1 reads device 1 from slot 3 of the system bus
  object [OFW-4.93 lv1 a:0x235DA4-0x235DAC]
  [OFW-4.93 lv1 a:0x235DC8-0x235DE8], the doubleword at object + 56
  [OFW-4.93 lv1 a:0x22B838-0x22B844]. lv1's init fills that slot through
  the same constructor as slot 0 [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A70]
  [OFW-4.93 lv1 a:0x3A9758-0x3A9780].
- The open function does not call that device's open entry. It gets a
  controller object from slot 0 of an array
  [OFW-4.93 lv1 a:0x237758-0x23775C] [OFW-4.93 lv1 a:0x2377DC-0x2377E0]
  [OFW-4.93 lv1 a:0x23D0F8-0x23D104]. The TOC slot that the getter reads
  holds the array's address, 0x3682F8
  [OFW-4.93 lv1 a:0x353C88-0x353C8C].
- lv1's init makes the controller object. It keeps the system bus
  object at +8 and the slot index 3 at +16, and stores the object in
  slot 0 of the array [OFW-4.93 lv1 a:0x23DA60-0x23DAC4]
  [OFW-4.93 lv1 a:0x23DBC8-0x23DBD0]. A later init step keeps the device
  of that slot at +24 [OFW-4.93 lv1 a:0x3AB040-0x3AB068], and sets the
  state word at +240 to 1 [OFW-4.93 lv1 a:0x3AB2E8].
- The open calls the controller's open
  [OFW-4.93 lv1 a:0x2377E4-0x2377E8]. Under the controller's lock, that
  open calls the device's entry at +16
  [OFW-4.93 lv1 a:0x23DC54-0x23DC74], which is the open entry that
  device 0 uses [OFW-4.93 lv1 a:0x2280A0-0x2280BC]. When the entry
  returns 0 and the device is open for the caller
  [OFW-4.93 lv1 a:0x23DCC0-0x23DCEC] [OFW-4.93 lv1 a:0x228120-0x228144],
  it sets the state word to 2 and returns 0
  [OFW-4.93 lv1 a:0x23DD14-0x23DD28].
- The close sets the state word to 3
  [OFW-4.93 lv1 a:0x23D880-0x23D8A0].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| system bus object pointer | 0x3682C0 | 0x61D710 |
| object + 56, slot 3 | 0x61D748 | 0x61E050, the device |
| device + 0, method table | 0x61E050 | 0x35C878, the constructor's table |
| array slot 0 | 0x3682F8 | 0x7D920, the controller |
| controller + 24 | 0x7D938 | 0x61E050, the device of slot 3 |
| controller + 16, slot index | 0x7D930 | 3 |
| controller + 240, state | 0x7DA10 | 2 |
| device + 108, open flag | 0x61E0BC | 0x01 |
| device + 112, partition | 0x61E0C0 | 2 |

- The device in slot 3 has the constructor's method table, and the
  controller holds that device. So the controller is the object whose
  open path the fix reads.
- The state word is 2. The controller's open sets 2 after the device's
  entry returned 0, and the controller's close sets 3. So the last of
  the two to run was the open.
- The open flag is 1 and the partition is 2. So the last open of the
  device was for the game OS partition, and no close followed it.

## Device 2

The 4.93 kernel reads the id of device 2 of bus 1 after device 1
[OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490], and opens
the device through the same call site
[OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718]. lv1 takes
the path of device 1 with the next slot and the next controller:

- The lookup reads device 2 from slot 4, the doubleword at object + 64
  [OFW-4.93 lv1 a:0x235DB0-0x235DB8] [OFW-4.93 lv1 a:0x22B838-0x22B844].
  lv1's init fills that slot through the same constructor
  [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AC4] [OFW-4.93 lv1 a:0x3A9758-0x3A9780].
- The open function gets the controller from slot 1 of the array and
  calls the controller's open with the caller's partition
  [OFW-4.93 lv1 a:0x237760-0x23777C].
- lv1's init makes that controller with the slot index 4 at +16, and
  stores it in slot 1 of the array [OFW-4.93 lv1 a:0x23DB28-0x23DB54]
  [OFW-4.93 lv1 a:0x23DBD4-0x23DBDC]. The init step of device 1 keeps
  the device of slot 4 at +24 [OFW-4.93 lv1 a:0x23DBB0]
  [OFW-4.93 lv1 a:0x3AB040-0x3AB068].

The walk on the reference console:

| Step | Address | Read |
|---|---|---|
| system bus object pointer | 0x3682C0 | 0x61D710 |
| object + 64, slot 4 | 0x61D750 | 0x61E2A0, the device |
| device + 0, method table | 0x61E2A0 | 0x35C878, the constructor's table |
| array slot 1 | 0x368300 | 0x7DB70, the controller |
| controller + 24 | 0x7DB88 | 0x61E2A0, the device of slot 4 |
| controller + 16, slot index | 0x7DB80 | 4 |
| controller + 240, state | 0x7DC60 | 2 |
| device + 108, open flag | 0x61E30C | 0x01 |
| device + 112, partition | 0x61E310 | 2 |

The values say what the values of device 1 say: the controller holds the
device of slot 4, its last open succeeded, and the device is open for
the game OS partition.

## Files

| File | Rows | Holds |
|---|---|---|
| `walk.tsv` | 3 | the doublewords at the system bus object pointer, at slot 0 and at the device's +0 |
| `open_flag.tsv` | 1 | the byte at the device's +108 |
| `partition.tsv` | 1 | the doubleword at the device's +112 |
| `device1_walk.tsv` | 3 | the doublewords at the system bus object pointer, at slot 3 and at device 1's +0 |
| `device1_controller.tsv` | 2 | the doublewords at array slot 0 and at the controller's +24 |
| `device1_controller_state.tsv` | 2 | the words at the controller's +16 and +240 |
| `device1_open_flag.tsv` | 1 | the byte at device 1's +108 |
| `device1_partition.tsv` | 1 | the doubleword at device 1's +112 |
| `device2_walk.tsv` | 3 | the doublewords at the system bus object pointer, at slot 4 and at device 2's +0 |
| `device2_controller.tsv` | 2 | the doublewords at array slot 1 and at the controller's +24 |
| `device2_controller_state.tsv` | 2 | the words at the controller's +16 and +240 |
| `device2_open_flag.tsv` | 1 | the byte at device 2's +108 |
| `device2_partition.tsv` | 1 | the doubleword at device 2's +112 |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x3682C0,0x61D730,0x61D900 --out walk.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x61D96C --out open_flag.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61D970 --out partition.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x3682C0,0x61D748,0x61E050 --out device1_walk.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x3682F8,0x7D938 --out device1_controller.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x7D930,0x7DA10 --out device1_controller_state.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x61E0BC --out device1_open_flag.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61E0C0 --out device1_partition.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x3682C0,0x61D750,0x61E2A0 --out device2_walk.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x368300,0x7DB88 --out device2_controller.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x7DB80,0x7DC60 --out device2_controller_state.tsv
cellgov ps3 peek --space lv1 --width 1 --address 0x61E30C --out device2_open_flag.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61E310 --out device2_partition.tsv
```
