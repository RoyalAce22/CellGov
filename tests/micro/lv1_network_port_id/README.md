# lv1_network_port_id

A read-only capture, in lv1 on the reference console, of the event
receive port that lv1 connects to the network device's interrupt. It
gives the port id that lv1's `construct_event_receive_port` (hypercall
18) gave the 4.93 kernel's call at
[OFW-4.93 lv2_kernel a:0x80000000000a28e0].

CellGov's hypervisor answers that call with this id through
`GAME_OS_PORT_CALLS` in `cellgov_ps3_abi::lv1::event_port`. The test
`event_port_network_console_tests` holds the row to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-17, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. Each read took the address that the
  read before it gave. It read each page twice, and the two reads of
  every word agree. It wrote nothing to lv1 or lv2, used no poke, sent
  no hypercall and installed nothing on the console.
- No row of `tests/micro/console_crashes.tsv` covers these addresses.
- Only the 15 words below are committed.
- No console-identifying value is in this directory.

The kernel keeps the id at +72 of its network device object
[OFW-4.93 lv2_kernel a:0x80000000000a28fc]. This capture does not read
that copy: a read of that object stopped the console (see
`tests/micro/console_crashes.md`).

## The kernel's calls

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps of the function at
0x80000000000a28a8:

- The network driver's `net_set_interrupt_status_indicator` success
  path calls the function with the network device object
  [OFW-4.93 lv2_kernel a:0x80000000000a3548]. The function keeps that
  object in r31 [OFW-4.93 lv2_kernel a:0x80000000000a28b4].
- It sends hypercall 18, and on r3 = 0 stores r4 at +72 of the object
  [OFW-4.93 lv2_kernel a:0x80000000000a28dc-0x80000000000a28fc].
- It gives +72 to the function at 0x8000000000013e1c
  [OFW-4.93 lv2_kernel a:0x80000000000a2900-0x80000000000a2904]. That
  function gives the id to the kernel's connect step
  [OFW-4.93 lv2_kernel a:0x8000000000013e34-0x8000000000013e48], which
  sends `connect_irq_plug_ext` (hypercall 12) with the id in r6
  [OFW-4.93 lv2_kernel a:0x8000000000008794-0x80000000000087f8].
- It sends `connect_interrupt_event_receive_port` (hypercall 197) with
  the bus id at +16 of the object in r3, the device id at +8 in r4, the
  id at +72 in r5 and 0 in r6
  [OFW-4.93 lv2_kernel a:0x80000000000a291c-0x80000000000a2930].

The network device is device 0 of bus 1; see
`cellgov_ps3_abi::lv1::network`.

## Where lv1 keeps the port

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path of hypercall 197. The TOC slot
r2 - 26632 is at 0x353830 (see `cellgov_ps3_abi::lv1::network`), so
lv1's r2 is 0x35A038.

- The handler [OFW-4.93 lv1 a:0x305CF0] refuses a bus id above 4
  [OFW-4.93 lv1 a:0x305D88-0x305D8C]. It gives the bus id, the device
  id, the port id and r6 to the function [OFW-4.93 lv1 a:0x236924]
  [OFW-4.93 lv1 a:0x305D94-0x305DB0].
- For bus 1 [OFW-4.93 lv1 a:0x236984-0x236988], device 0 and an r6 of
  0, that function takes the network device from the global object of
  the TOC slot r2 - 26632 and checks its state word
  [OFW-4.93 lv1 a:0x236A0C-0x236A68].
- On state 2 it loads the connection object from the TOC slot
  r2 - 26624, and gives it and the port id to the connect function
  [OFW-4.93 lv1 a:0x236BB4-0x236BC0]. The slot at 0x353838 holds
  0x3682E8, the global that holds the connection object.
- The connect function returns -9 (`LV1_BUSY`) while +32 of the
  connection object holds a port whose word at +8, shifted right by 2,
  is not 0 [OFW-4.93 lv1 a:0x245F88-0x245FB0].
- It looks up the port id in the calling partition's object table
  [OFW-4.93 lv1 a:0x245FFC-0x246108]. It checks the object's class word
  at +48 against the TOC slot r2 - 24632, which holds 0x329310
  [OFW-4.93 lv1 a:0x246114-0x246120].
- It gives the port and the handle at +16 of the connection object to
  the link function [OFW-4.93 lv1 a:0x246148-0x246150]. The link
  function stores the port at +16 of the handle
  [OFW-4.93 lv1 a:0x2BEDD4]. So +32 of the connection object holds the
  connected port.

## Where a port keeps its id

- The object constructor gives the table's allocator the address of
  +72 of the object [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC]. The allocator
  stores the new id there [OFW-4.93 lv1 a:0x2CA42C]. The constructor
  then reads the id back from +72 [OFW-4.93 lv1 a:0x2CC700].
- The object constructor stores the class at +48 and the partition at
  +56 [OFW-4.93 lv1 a:0x2CC6A8-0x2CC6B0].
- The port constructor stores the partition at +120, and the word that
  `construct_event_receive_port` gives it at +136; see
  `cellgov_ps3_abi::lv1::event_port` and `cellgov_ps3_abi::lv1::send_event`.

## Where the table keeps an id

The table is at +88 of the partition [OFW-4.93 lv1 a:0x245FFC-0x246008].

- +24 of the table holds the array of segments.
- For an id below 1 << (the word at +32 of the table), the slot is in
  segment 0 [OFW-4.93 lv1 a:0x24609C-0x2460CC].
- The slot of an id in segment 0 is the word at +72 of the segment plus
  the id times the word at +48. The slot holds the object
  [OFW-4.93 lv1 a:0x2460F4-0x246108].

## Files

| File | Rows | Holds |
|---|---|---|
| `network_port.tsv` | 15 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields:

- `connection`: the global at 0x3682E8, the connection object.
- `port`: +32 of the connection object.
- `port_class`, `port_partition_56`, `port_id`, `port_partition_120`,
  `port_connection`, `port_word` and `port_plug`: the port's words at
  +48, +56, +72, +120, +128, +136 and +144.
- `table_segments` and `table_shape`: +24 and +32 of the partition's
  table.
- `segment_0`, `segment_stride` and `segment_base`: the first word of
  the segment array, and +48 and +72 of that segment.
- `slot_19`: the slot of id 19 in segment 0.

## The live port

- The connection object holds the port at 0x38A830.
- The port's class word is 0x329310, the class that the connect
  function checks. Its word at +136 is 0x3298D2, the word that the TOC
  slot r2 - 7896 holds
  [OFW-4.93 lv1 a:0x2E2538]. So the object is a port of
  `construct_event_receive_port`.
- Both partition words hold 0x91390, the partition of the port.
- The word at +128 is not 0, so the port has a connection. The word at
  +144 holds 5, the bit number of the connected plug, then the byte
  1 at +148 and the pending byte 0 at +149.
- +72 of the port holds 19 (0x13), the port id.
- The partition's table has 64 slots in segment 0 (the word 6 at +32),
  each 8 bytes. The slot of id 19 holds 0x38A830, the same port. So lv1
  gives the network device's interrupt port the id 19.
- The read gives the port connected at the time of the read. The
  capture does not tell which hardware thread made the call; the id
  comes from the partition's table, so the thread does not change it.
- lv1 gives a new port the first free id of the partition's object
  table, so ids 0 to 18 were in use at the time of the call. The
  capture `lv2_record_port_ids` gives 17 to thread 0's record call. No
  capture names the object at id 18.
