# lv1_storage_notify_request

A read-only capture, in lv1 on the reference console, of the
notification requests that lv1 holds on the storage bus. It gives the
lv1 state that the answer to the 4.93 kernel's `storage_write`
(hypercall 246) to the notification device reads
[OFW-4.93 lv2_kernel a:0x80000000000ae250]:

- the state of the storage bus, which decides that lv1 completes the
  request in the call;
- the word that lv1 gives each new request;
- the request tags that the other partition holds;
- the event mask that the call stores for the game OS partition.

CellGov's hypervisor answers that call through
`cellgov_ps3_abi::lv1::storage`. The test `notify_console_tests` holds
the answer to this capture.

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
- Only the 28 words below are committed.
- No console-identifying value is in this directory.

## The kernel's call

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The storage bus function at 0x80000000000ae0c0 zeroes a 512-byte
  buffer, and stores 0 at +0 and 2 at +8 of it
  [OFW-4.93 lv2_kernel a:0x80000000000ae1dc-0x80000000000ae204].
- It sends hypercall 246 with r3 = -1, r4 = 0, r5 = 0, r6 = 1, r7 = 0
  and r8 = the buffer's address
  [OFW-4.93 lv2_kernel a:0x80000000000ae234-0x80000000000ae250].
- On r3 = 0 it keeps r4 as the request's tag
  [OFW-4.93 lv2_kernel a:0x80000000000ae254-0x80000000000ae278]. A
  nonzero r3 makes the function return 0x8001000C
  [OFW-4.93 lv2_kernel a:0x80000000000ae30c-0x80000000000ae324].

## lv1's path

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. The storage bus object is S, the
object at the global 0x368388 (see the capture `lv1_storage_bus_open`).

- The handler [OFW-4.93 lv1 a:0x308A08] zeroes the return slot of r4.
  It calls the write core with the caller's partition, r3 to r8 and the
  slot's address [OFW-4.93 lv1 a:0x308AB4-0x308AF0]. It returns the
  core's r3 zero-extended [OFW-4.93 lv1 a:0x308B58].
- The core reads S from the global that the TOC slot at r2 - 23408
  locates [OFW-4.93 lv1 a:0x251340-0x251360]. The slot holds 0x368388
  [OFW-4.93 lv1 a:0x3544C8-0x3544CC].
- For device -1 the core uses S as the device
  [OFW-4.93 lv1 a:0x251434] [OFW-4.93 lv1 a:0x2514C0-0x2514C4]. It asks
  S's ready method, slot 6 of the method table
  [OFW-4.93 lv1 a:0x2513B8-0x2513CC]. The method returns true when a
  slot at +2544 or +2608 holds the partition with its used byte set
  [OFW-4.93 lv1 a:0x25477C-0x2547FC]. Otherwise the core returns -11
  [OFW-4.93 lv1 a:0x2513F8-0x251400].
- The core then calls S's write method, slot 33
  [OFW-4.93 lv1 a:0x251470-0x251484]. S's method table is 0x35DD30;
  slot 33 holds the method at 0x25A664
  [OFW-4.93 lv1 a:0x35DE38-0x35DE3C] [OFW-4.93 lv1 a:0x3356A0-0x3356A4].
- The write method returns -17 unless the start sector is 0, the
  sector count is 1 and the flags are 0
  [OFW-4.93 lv1 a:0x25A664-0x25A6B8]. It does not read the region id.
- It translates the buffer's 512 bytes [OFW-4.93 lv1 a:0x25A6F0-0x25A708]
  [OFW-4.93 lv1 a:0x254484-0x2545A0]. It returns -17 unless the
  doubleword at +0 is 0, and keeps the doubleword at +8 as the event
  mask [OFW-4.93 lv1 a:0x25A70C-0x25A720].
- For a partition other than 1, it takes the lock at +3576, asks the
  ready method again, and walks the list of open records at +3552
  [OFW-4.93 lv1 a:0x25A728-0x25A7AC]. It stores the event mask at +48
  of the partition's record [OFW-4.93 lv1 a:0x25A8CC-0x25A8D4]. For
  partition 1 it stores 5 [OFW-4.93 lv1 a:0x25A8D8-0x25A8E0].
- Under the lock at +2672 it calls the request function with the
  slot's address, the partition, the buffer's address and the word at
  +3464 [OFW-4.93 lv1 a:0x25A7CC-0x25A80C].
- The request function returns -9 when the 16 bits of the request
  bitmap at +2680 are all set [OFW-4.93 lv1 a:0x25398C-0x2539CC]. For a
  word of 0 at +3464 it takes no memory object
  [OFW-4.93 lv1 a:0x2539D0-0x2539D8]. It sets the lowest clear bit, and
  stores its number in the slot as the tag
  [OFW-4.93 lv1 a:0x2539DC-0x253A04]. It fills the 48-byte entry of the
  tag at +2688: the partition at +0, the memory object at +8, the word
  at +16 and 0 at +32 [OFW-4.93 lv1 a:0x253A08-0x253A50].
- The write method then calls the completion function with the tag and
  status 0 [OFW-4.93 lv1 a:0x25A82C-0x25A838]. When the state word at
  +3456 is 4, it stores the status at +24 of the entry and the byte 1
  at +40 [OFW-4.93 lv1 a:0x256464-0x256494]. It finds the slot that
  holds the entry's partition and the port at +2584 of that slot. For a
  port that holds a reference, it calls the event function with that
  port [OFW-4.93 lv1 a:0x256498-0x25654C]. The event function
  [OFW-4.93 lv1 a:0x2DF808] sets the port's pending byte and raises the
  plug's bit.

So the call answers r3 = 0 and r4 = the tag. lv1 keeps a completed
request under the tag, and the partition's port gets an event.

## The values

| Read | Address | Value | What it gives |
|---|---|---|---|
| S | 0x368388 | 0x7DDC0 | the storage bus object of `lv1_storage_bus_open` |
| S + 3456 | 0x7EB40 | word 4 | the state that completes a request in the call |
| S + 3464 | 0x7EB48 | 0 | no memory object for a new request |
| S + 2680 | 0x7E838 | 0x3 | tags 0 and 1 held |
| entry 0 + 0 | 0x7E840 | 1 | tag 0 is partition 1's |
| entry 1 + 0 | 0x7E870 | 2 | tag 1 is the game OS partition's |
| entries + 8 to + 40 | | 0 | no memory object, and both requests wait |
| list + 0, first record | 0x7EBA0 | 0x6B5700 | partition 1's record |
| list + 8, last record | 0x7EBA8 | 0x38B590 | the game OS partition's record |
| list + 16, count | 0x7EBB0 | 2 | two open records |
| first record + 40, + 48 | 0x6B5728 | 1, then 5 | partition 1, with the mask that lv1 sets for it |
| last record + 40, + 48 | 0x38B5B8 | 2, then 2 | the game OS partition, with the kernel's mask 2 |
| slot 0 + 0, + 40 | 0x7E7B0 | 1, then 0x6B3ED0 | partition 1 and its port |
| slot 1 + 0, + 40 | 0x7E7F0 | 2, then 0x38BB20 | the game OS partition and the port of `lv1_storage_port_id` |
| port + 8 | 0x38BB28 | word 4 | one reference (bits 2 to 31) |

- The state word is 4, so lv1 completes a notification request in the
  call.
- The word at +3464 is 0, so a request takes no memory object.
- Both entries wait with no completion. Each partition holds one
  request at the time of the read: the request that waits for the next
  device event.
- The game OS partition's record holds the mask 2 that the kernel's
  call stores.
- The port of the game OS partition's slot holds a reference, so the
  event function runs for it.

## The tag

lv1 gives a new request the lowest clear bit of the bitmap. At the time
of the read, partition 1 holds tag 0 and the game OS partition holds
tag 1. The model gives the kernel's first request tag 1, from these
facts:

- Partition 1 opened the storage bus first: it holds slot 0 and the
  first record. So it made its own requests before the game OS
  partition booted.
- A partition holds its request until a device event completes it, and
  then makes the next one. So partition 1 holds a tag from its first
  request on, except in the short time between two requests.

The read cannot show the bitmap at the time of the kernel's call. A
completion of partition 1's request at that time would give the kernel
tag 0.

## Files

| File | Rows | Holds |
|---|---|---|
| `bus.tsv` | 1 | the storage bus object pointer at 0x368388 |
| `request.tsv` | 3 | +3456, +3464 and +2680 of the bus object |
| `entries.tsv` | 12 | +0, +8, +16, +24, +32 and +40 of the entries of tags 0 and 1 |
| `list.tsv` | 3 | +0, +8 and +16 of the list of open records at +3552 |
| `records.tsv` | 4 | +40 and +48 of the first and the last record |
| `slots.tsv` | 4 | +0 and +40 of slots 0 and 1, at +2544 and +2608 |
| `port_ref.tsv` | 1 | +8 of the port in slot 1 |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x368388 --out bus.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EB40,0x7EB48,0x7E838 --out request.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7E840,0x7E848,0x7E850,0x7E858,0x7E860,0x7E868,0x7E870,0x7E878,0x7E880,0x7E888,0x7E890,0x7E898 --out entries.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EBA0,0x7EBA8,0x7EBB0 --out list.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x6B5728,0x6B5730,0x38B5B8,0x38B5C0 --out records.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7E7B0,0x7E7D8,0x7E7F0,0x7E818 --out slots.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x38BB28 --out port_ref.tsv
```
