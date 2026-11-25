# lv1_storage_port_id

A read-only capture, in lv1 on the reference console, of the event
receive port that lv1 connects to the storage bus for the game OS
partition. It gives the port id that lv1's `construct_event_receive_port`
(hypercall 18) gave the 4.93 kernel's call at
[OFW-4.93 lv2_kernel a:0x8000000000096494].

CellGov's hypervisor answers that call with this id through
`GAME_OS_PORT_CALLS` in `cellgov_ps3_abi::lv1::event_port`. The test
`event_port_storage_console_tests` holds the row to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-18, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. Each read took the address that the
  read before it gave. It read each page twice, and the two reads of
  every word agree. It wrote nothing to lv1 or lv2, used no poke, sent
  no hypercall and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range.
- Only the 20 words below are committed.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The storage bus function at 0x80000000000ae0c0 opens bus 4, device -1
  with `open_device` (hypercall 170) and goes on only when r3 is 0
  [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae118]. The
  capture `lv1_storage_bus_open` reads that open.
- It allocates a 96-byte interrupt object
  [OFW-4.93 lv2_kernel a:0x80000000000ae120-0x80000000000ae13c]. It sets
  the object up with type 2, interrupt id 0, the bus id at +8 of the bus
  object, and device -1
  [OFW-4.93 lv2_kernel a:0x80000000000ae140-0x80000000000ae150]. The
  setup stores the device at +88, the type at +16, the interrupt id at
  +24 and the bus at +80
  [OFW-4.93 lv2_kernel a:0x8000000000096334-0x8000000000096350].
- It calls the register function at 0x800000000009655c with the object
  [OFW-4.93 lv2_kernel a:0x80000000000ae170]. That function calls the
  port setup at 0x80000000000963f4
  [OFW-4.93 lv2_kernel a:0x80000000000965f0].
- The port setup sends hypercall 18 for an object of type 2
  [OFW-4.93 lv2_kernel a:0x800000000009641c-0x8000000000096424]
  [OFW-4.93 lv2_kernel a:0x8000000000096490-0x8000000000096494]. On
  r3 = 0 it keeps r4
  [OFW-4.93 lv2_kernel a:0x8000000000096498-0x80000000000964a4].
- It then sends `connect_interrupt_event_receive_port` (hypercall 197)
  with the bus at +80 in r3, the device at +88 in r4, the id in r5 and
  the interrupt id at +24 in r6
  [OFW-4.93 lv2_kernel a:0x80000000000964b8-0x80000000000964cc].

The port setup serves each interrupt object of type 2, so a later
driver's object can reach the same `sc`.

## Where lv1 keeps the port

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path of hypercall 197 for the storage
bus:

- For bus 4, lv1 reads the storage bus object from the global at
  0x368388; see the capture `lv1_storage_bus_open`.
- The storage connect looks up the port id in the calling partition's
  object table, and checks the object's class word at +48 against the
  TOC slot r2 - 23160 [OFW-4.93 lv1 a:0x2553A0-0x2553CC]. That slot
  holds 0x329310, the class of a port.
- When the slot at +2608 of the bus object holds the caller's
  partition, the connect takes slot 1. It refuses with -9 (`LV1_BUSY`)
  while +2648 holds a port whose word at +8, shifted right by 2, is not
  0 [OFW-4.93 lv1 a:0x255508-0x255548].
- It gives the port and the handle at +2632 of the bus object to the
  link function [OFW-4.93 lv1 a:0x25554C-0x255570]. The link function
  stores the port at +16 of the handle [OFW-4.93 lv1 a:0x2BEDD4]. So
  +2648 of the bus object holds the connected port.

The capture `lv1_storage_bus_open` shows that slot 1 holds partition 2,
the game OS partition.

## Where the table keeps an id

The capture `lv1_network_port_id` gives the table's layout: the table is
at +88 of the partition, +24 of the table holds the segment array, and
the slot of an id in segment 0 is the word at +72 of the segment plus
the id times the word at +48.

lv1's allocator [OFW-4.93 lv1 a:0x2CA258] gives a new object the lowest
free id:

- It walks the free bitmap at +64 of the segment, whose slot count is
  at +56, from doubleword 0 [OFW-4.93 lv1 a:0x2CA3AC-0x2CA3D8].
- In the first nonzero doubleword it clears the lowest set bit
  [OFW-4.93 lv1 a:0x2CA3DC-0x2CA3F4].
- The id is 64 times the doubleword index plus the bit index, with bit
  0 the least significant, plus the segment base
  [OFW-4.93 lv1 a:0x2CA410-0x2CA42C].

So a set bit is a free id.

## Files

| File | Rows | Holds |
|---|---|---|
| `bus.tsv` | 1 | the storage bus object pointer at 0x368388 |
| `bus_port.tsv` | 1 | +2648 of the bus object, the connected port |
| `port.tsv` | 7 | the port's words at +48, +56, +72, +120, +128, +136 and +144 |
| `table.tsv` | 8 | +24 and +32 of the partition's table, the first segment, +48 and +72 of that segment, and the slots of ids 0, 19 and 20 |
| `segment.tsv` | 2 | +56 and +64 of the first segment |
| `bitmap.tsv` | 1 | doubleword 0 of the first segment's free bitmap |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x368388 --out bus.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7E818 --out bus_port.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x38BB50,0x38BB58,0x38BB68,0x38BB98,0x38BBA0,0x38BBA8,0x38BBB0 --out port.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x91400,0x91408,0x12DA50,0x91260,0x91278,0x12DB80,0x12DC18,0x12DC20 --out table.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x91268,0x91270 --out segment.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0xAB300 --out bitmap.tsv
```

## The live port

- The storage bus object is at 0x7DDC0, the object of the capture
  `lv1_storage_bus_open`. Its +2648 holds the port at 0x38BB20.
- The port's class word is 0x329310, the class that the storage connect
  checks. Its word at +136 is 0x3298D2, the word of a port of
  `construct_event_receive_port`; see the capture
  `lv1_network_port_id`.
- Both partition words hold 0x91390, the partition of the network
  capture's port.
- The word at +128 is not 0, so the port has a connection. It is the
  connection of the network capture's port. The word at +144 holds 6,
  the bit number of the connected plug, then the byte 1 at +148 and the
  pending byte 0 at +149.
- +72 of the port holds 20 (0x14), the port id.
- The table's words match the network capture: 64 slots in segment 0
  (the word 6 at +32), each 8 bytes, from 0x12DB80. The slot of id 20
  holds 0x38BB20, the same port. The slot of id 19 still holds the
  network capture's port at 0x38A830. So lv1 gives the storage bus's
  interrupt port the id 20, the id after the network device's port.
- The read gives the port connected at the time of the read. The
  capture does not tell which hardware thread made the call; the id
  comes from the partition's table, so the thread does not change it.

## The id 0

- The slot of id 0 holds 0, so no object is in it.
- Segment 0 has 64 slots (0x40 at +56). Doubleword 0 of its free bitmap
  is 0xFE00000000000000. Bits 0 to 56 are clear, so ids 0 to 56 are in
  use at the time of the read, and ids 57 to 63 are free.
- So id 0 is in use with no object in its slot. The read does not tell
  which owner holds it. Its bit was clear at the network device's call
  too, since that call got 19.
