# lv1_storage_device_port_ids

A read-only capture, in lv1 on the reference console, of the event
receive port that lv1 connects to each storage device for the game OS
partition. It gives the port id that lv1's
`construct_event_receive_port` (hypercall 18) gave each of the 4.93
kernel's calls at [OFW-4.93 lv2_kernel a:0x8000000000096494] after the
call of the storage bus.

CellGov's hypervisor answers those calls with these ids through
`GAME_OS_PORT_CALLS` in `cellgov_ps3_abi::lv1::event_port`. The test
`event_port_storage_device_console_tests` holds the rows to this
capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48.
- On 2026-11-19, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. Each read took the address that the
  read before it gave. It read each page twice, and the two reads of
  every word agree. It wrote nothing to lv1 or lv2, used no poke, sent
  no hypercall and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range.
- Only the 74 words below are committed.
- No console-identifying value is in this directory.

## The kernel's calls

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The kernel's device object opens device +152 of bus +144 with
  `open_device` (hypercall 170)
  [OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac120]. It goes
  on only when r3 is 0
  [OFW-4.93 lv2_kernel a:0x80000000000ac124-0x80000000000ac12c]. The
  capture `lv1_storage_device_open` shows the game OS partition's open
  of each of the five storage devices.
- It sets up a 96-byte interrupt object with type 2, interrupt id 0,
  the bus at +144 and the device at +152
  [OFW-4.93 lv2_kernel a:0x80000000000ac154-0x80000000000ac164].
- It gives the object to the register function at 0x800000000009655c
  [OFW-4.93 lv2_kernel a:0x80000000000ac1ec]. That function calls the
  port setup at 0x80000000000963f4
  [OFW-4.93 lv2_kernel a:0x80000000000965f0].
- The port setup sends hypercall 18 for an object of type 2
  [OFW-4.93 lv2_kernel a:0x800000000009641c-0x8000000000096424]
  [OFW-4.93 lv2_kernel a:0x8000000000096490-0x8000000000096494]. It then
  sends `connect_interrupt_event_receive_port` (hypercall 197) with the
  bus at +80 in r3, the device at +88 in r4 and the port id in r5
  [OFW-4.93 lv2_kernel a:0x80000000000964b8-0x80000000000964cc].

So each storage device's open reaches the same `sc` as the storage bus
function's call. The capture `lv1_storage_port_id` gives the id of
that first call.

## Where lv1 keeps the port

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path of hypercall 197 for a storage
device. D is the device object.

- For a device id up to 6, the connect function gives D, the
  doubleword at +3816 + 8 * id of the storage bus object, to the
  storage connect [OFW-4.93 lv1 a:0x23699C-0x2369BC].
- The storage connect loads the calling partition
  [OFW-4.93 lv1 a:0x2552A8-0x2552B0]. It compares the word at +80 of
  that partition with the partition word of D's slot 0 at +2544, then
  with that of slot 1 at +2608, and takes the first used slot that
  holds it [OFW-4.93 lv1 a:0x2554F0-0x25551C]
  [OFW-4.93 lv1 a:0x2555F8-0x255608].
- It gives the port and the slot's handle at +2568 + 64 * slot to the
  link function [OFW-4.93 lv1 a:0x25554C-0x255570]. The link function
  stores the port at +16 of the handle [OFW-4.93 lv1 a:0x2BEDD4].

So D + 2584 + 64 * slot holds the port connected for the partition of
that slot.

## Where the table keeps an id

The capture `lv1_storage_port_id` gives the table's layout: the table
is at +88 of the partition, +24 of the table holds the segment array,
and the slot of an id in segment 0 is the word at +72 of the segment
plus the id times the word at +48. An object's id is at its +72
[OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].

lv1's allocator gives a new object the lowest free id of the table
[OFW-4.93 lv1 a:0x2CA3DC-0x2CA42C]. A set bit of the free bitmap at
+64 of the segment is a free id.

## Files

| File | Rows | Holds |
|---|---|---|
| `bus.tsv` | 1 | the storage bus object pointer at 0x368388 |
| `devices.tsv` | 5 | the device objects D at +3816 + 8 * id of the bus object, ids 0 to 4 |
| `slots.tsv` | 10 | the partition words of slots 0 and 1 of each device, at +2544 and +2608 |
| `device_port.tsv` | 10 | the ports of slots 0 and 1 of each device, at +2584 and +2648 |
| `port.tsv` | 35 | the words at +48, +56, +72, +120, +128, +136 and +144 of each game OS port |
| `partition.tsv` | 1 | +80 of the ports' partition |
| `table.tsv` | 11 | +24 and +32 of the partition's table, the first segment, +48, +56 and +72 of that segment, and the slots of ids 21 to 25 |
| `bitmap.tsv` | 1 | doubleword 0 of the first segment's free bitmap |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x368388 --out bus.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7ECA8,0x7ECB0,0x7ECB8,0x7ECC0,0x7ECC8 --out devices.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x628A00,0x628A40,0x62A3C0,0x62A400,0x660A00,0x660A40,0x665F90,0x665FD0,0x368D88,0x368DC8 --out slots.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x628A28,0x628A68,0x62A3E8,0x62A428,0x660A28,0x660A68,0x665FB8,0x665FF8,0x368DB0,0x368DF0 --out device_port.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x38BCC0,0x38BCC8,0x38BCD8,0x38BD08,0x38BD10,0x38BD18,0x38BD20,0x38BE30,0x38BE38,0x38BE48,0x38BE78,0x38BE80,0x38BE88,0x38BE90,0x538040,0x538048,0x538058,0x538088,0x538090,0x538098,0x5380A0,0x538150,0x538158,0x538168,0x538198,0x5381A0,0x5381A8,0x5381B0,0x53E050,0x53E058,0x53E068,0x53E098,0x53E0A0,0x53E0A8,0x53E0B0 --out port.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x913E0 --out partition.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x91400,0x91408,0x12DA50,0x91260,0x91268,0x91278,0x12DC28,0x12DC30,0x12DC38,0x12DC40,0x12DC48 --out table.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0xAB300 --out bitmap.tsv
```

## The live ports

- The storage bus object is at 0x7DDC0, and its device objects are
  those of the capture `lv1_storage_device_open`.
- Slot 0 of devices 0, 2 and 3 and slot 1 of devices 1 and 4 hold the
  partition word 2. +80 of the ports' partition 0x91390 holds 2. So
  those slots are the game OS partition's.
- The game OS slot of each device holds a port: 0x38BC90, 0x38BE00,
  0x538010, 0x538120 and 0x53E020 for devices 0 to 4. Slot 0 of devices
  1 and 4 holds the port 0x6B3ED0 of the other partition. The other
  slots hold 0.
- Each port's class word at +48 is 0x329310, the class that the storage
  connect checks, and its word at +136 is 0x3298D2, the word of a port
  of `construct_event_receive_port`; see the capture
  `lv1_storage_port_id`. Both partition words of each port hold
  0x91390.
- Each port's +72 holds its id: 21, 22, 23, 24 and 25 for devices 0 to
  4.
- The table holds 64 slots in segment 0, each 8 bytes, from 0x12DB80.
  The slots of ids 21 to 25 hold the five ports. Doubleword 0 of the
  free bitmap is 0xFE00000000000000, so ids 0 to 56 are in use.
- The ids follow the storage bus port's id 20 with no gap. lv1 gives
  the lowest free id each time, so the device opens made their calls
  in the order of the device id. No object that the table still holds
  took an id between them.
- The read gives the ports connected at the time of the read. The
  capture does not tell which hardware thread made each call; the id
  comes from the partition's table, so the thread does not change it.
