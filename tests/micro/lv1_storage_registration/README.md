# lv1_storage_registration

A read-only capture, in lv1 on the reference console, of the storage
device registration that lv1 runs for the game OS partition. lv1 runs
it for each storage device that it reports to the game OS through the
notification device of the storage bus. The registration writes
repository nodes of lpar 1 that the 4.93 kernel reads after the report.

CellGov's hypervisor models the registration through
`cellgov_ps3_abi::lv1::storage_registration`. The tests of that module
hold the model to the files of this directory.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words that the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-18, `cellgov ps3 peek --space lv1 --width 8` read the
  doublewords below through webMAN's `peek.lv1` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads of every word agree. It wrote nothing to lv1 or lv2, used no
  poke, sent no hypercall and installed nothing on the console.
- Every address is lv1 heap or lv1 data. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range.
- The capture did not walk the repository again. The walk of
  `tests/micro/lv1_repository_list` gives the address of each node. For
  each node, the capture read the four key words at +64 to +88, v1 at
  +104 and v2 at +112. The four key words of every node agree with the
  key that `list.tsv` of that walk gives for the node's address.
- `node.tsv` and `region.tsv` keep each node's key and its two values.
  Their `address` column is the lv1 address of v1. The raw pages of the
  node reads are deleted.
- No console-identifying value is in this directory.

## lv1's path

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. S is the storage bus object, the
object at the global 0x368388 (see the capture `lv1_storage_bus_open`).
No read covers the body of the region access check's ACL comparison
[OFW-4.93 lv1 a:0x25B6F0-0x25B7C8], and no path below enters it.

Two paths run the registration 0x2524A4 for a device:

- The read method of the notification device takes a bit-1 event for a
  record whose partition is not 1. It calls 0x2524A4 with the word at
  S + 128 and the device id at +8 of the table entry, and only then
  writes the buffer and completes the request
  [OFW-4.93 lv1 a:0x259540-0x25955C].
- lv1's device-ready event raises S + 3792
  [OFW-4.93 lv1 a:0x258F2C-0x258F5C]. For each open record whose
  partition is not 1 it calls 0x2524A4 [OFW-4.93 lv1 a:0x258F80-0x258F94].
  Then it completes a waiting bit-1 read of that record
  [OFW-4.93 lv1 a:0x259004-0x259040]. lv1 calls the event with the lock
  at S + 3576 held [OFW-4.93 lv1 a:0x259750-0x25978C].

So on both paths lv1 registers a device before the game OS gets its
report.

The registration [OFW-4.93 lv1 a:0x2524A4-0x2526E4] does these steps for
the device object D at S + 3816 + 8 * id:

1. It calls the one-time initializer 0x251FC8 (see below).
2. It writes `bus.4.dev.<n>.id` = (id, 0) and `bus.4.dev.<n>.type` =
   (the type, 0), where n is the doubleword at D + 3536 and the type is
   the answer of D's method slot 37 [OFW-4.93 lv1 a:0x252504-0x252534]
   [OFW-4.93 lv1 a:0x23830C-0x23849C].
3. It writes `bus.4.dev.<n>.intr.0` = (2, 0)
   [OFW-4.93 lv1 a:0x252538-0x25254C] [OFW-4.93 lv1 a:0x2387F8-0x238960].
4. It writes the region nodes of the device's region list at
   +2288 of the object at D + 144 [OFW-4.93 lv1 a:0x25B16C-0x25B34C]:
   `region.<i>.id` = (i, 0), `region.<i>.size`, `region.<i>.start` and
   `region.<i>.crypto`, each with v2 = 0, for each region that it
   counts; then `bus.4.dev.<n>.n_regs` = (the count, 0).
5. It writes `blk_size` = (D + 3464, 0) and `n_blocks` = (D + 3472, 0),
   then `port`, `meddling` and `hosttype` from D's method slots 39, 31
   and 38, each with v2 = 0 [OFW-4.93 lv1 a:0x252558-0x25266C].
6. It writes `bus.4.id` = (4, 0), `bus.4.type` = (5, 0) and
   `bus.4.num_dev` = (S + 3808, 0) [OFW-4.93 lv1 a:0x252670-0x252680]
   [OFW-4.93 lv1 a:0x2380B8-0x2380C4] [OFW-4.93 lv1 a:0x237E8C-0x2380B4].
   The call site loads (4, 4, 5, S + 3808) into r3 to r6. The entry
   0x2380B8 moves r6 to r7 and r5 to r6, sets r5 to 0 and branches to
   the writer. The writer therefore gets the 5 as the type and
   S + 3808 as the device count.
7. It calls D's method slots 37 and 39 again and returns 0
   [OFW-4.93 lv1 a:0x252684-0x2526E4].

Each node write builds the key from the field names and the indices,
and then writes the node, or makes it when the repository holds no node
of that key [OFW-4.93 lv1 a:0x2384A0-0x238604]
[OFW-4.93 lv1 a:0x237BDC-0x237C54]. The repository is the object at the
global 0x373FB0, which the slot at r2 - 26600 locates
[OFW-4.93 lv1 a:0x237BE8-0x237C10] [OFW-4.93 lv1 a:0x353850-0x353854].

The initializer 0x251FC8 takes the word at S + 3880 from 0 to 1. When
the word is already set, it returns at once
[OFW-4.93 lv1 a:0x25201C-0x252048]. Otherwise, for each id from 0 to 6
that its check accepts, it writes placeholder nodes
[OFW-4.93 lv1 a:0x2520A0-0x252398]:

- `id` = (id, 0), `type` = (32, 0), `intr.0` = (2, 0);
- `blk_size`, `n_blocks`, `port`, `meddling`, `hosttype` and `n_regs` =
  (0, 0xDEADBEEF);
- for each region i from 0 to 7: `region.<i>.id` = (i, 0xDEADBEEF), and
  `start`, `size` and `crypto` = (0, 0xDEADBEEF).

Then it writes `bus.4.id` = (4, 0), `bus.4.type` = (5, 0) and
`bus.4.num_dev` = (0, 0) [OFW-4.93 lv1 a:0x25239C-0x2523AC]. No read
covers the check 0x2E49E8 that selects the ids.

For the five device objects, the method slots that the registration
calls return a value. Their bodies store nothing through the device
object or a global. Two bodies call further code that no read follows:
- slot 38, which calls one more method through a pointer;
- slot 31 of the device of type 5, which calls two functions.

The capture gives the values that the registration wrote, so the model
does not depend on those calls.

## The values

| Read | Address | Value | What it gives |
|---|---|---|---|
| S | 0x368388 | 0x7DDC0 | the storage bus object |
| S + 128 | 0x7DE40 | 0x7DDC0 | the object the registration gets is S |
| repository global | 0x373FB0 | 0x687060 | lpar 1's repository, from the walk of `lv1_repository_list` |
| S + 3552 list | 0x7EBA0 | 0x6B5700, 0x38B590, 2 | partition 1's record, the game OS record, two records |
| table | 0x7EBC8 | 5 entries | (bus, device, type, port) = (4,0,0xFF,0), (4,1,0xFE,0), (4,2,5,0), (4,3,0,2), (4,4,14,0) |
| S + 3784, + 3792 | 0x7EC88 | 5, 5 | five table entries and five device-ready events |
| S + 3808 | 0x7ECA0 | 5 | the `num_dev` value that the registration writes |
| S + 3816 + 8 * id | 0x7ECA8 | ids 0 to 4 set, 5 and 6 null | the device objects |
| D + 3536 | | n = id for ids 0 to 4 | the device index of each node key |
| D + 3464, D + 3472 | | see `device_fields.tsv` | `blk_size` and `n_blocks` of each device |
| S + 3872, S + 3880 | 0x7ECE0 | byte 1 at S + 3876, then 1 | the initializer ran |
| game OS record + 56 to + 80 | 0x38B5C8 | 0, 5, 0x3B3000, 1 | five bit-1 events taken, the sixth read waits under tag 1 |
| game OS record + 1392, + 1408 | 0x38BB00 | 0, 0 | no queued event |
| partition 1 record + 56 to + 80 | 0x6B5738 | 5, 0, 0x1D0000, 0 | five type-0 events taken |

`node.tsv` holds the nine nodes of each device 0 to 4 and
`bus.4.num_dev`. It also holds `type`, `blk_size` and `n_regs` of
devices 5 and 6, which lv1 does not report. `region.tsv` holds the
regions of devices 0 to 4 below `n_regs`, and the first region at or
above it.

- For devices 0 to 4, each node holds v2 = 0, the value that the
  registration writes. `id` is the device id, `type` is the table
  type, `intr.0` is (2, 0), and `blk_size` and `n_blocks` are the
  device object's words.
- For devices 5 and 6, and for each region at or above `n_regs`, the
  nodes hold the initializer's placeholders.
- `bus.4.num_dev` holds 5, the word at S + 3808.

## Timing

The registration model does not depend on the time at which each device
became ready: lv1 registers a device before each report on both paths.

The time matters for the answer to the kernel's first reads of the
notification device. The capture fits the assumption that all five
devices were ready before the game OS opened the bus: S + 3792 is 5,
the game OS record took five bit-1 events, and its sixth read waits. A
read after boot cannot show the order. With the assumption, the
kernel's first five reads complete at once, and the sixth waits.

## Files

| File | Rows | Holds |
|---|---|---|
| `bus.tsv` | 3 | S, S + 128 and the repository global |
| `list.tsv` | 3 | +0, +8 and +16 of the open record list at S + 3552 |
| `table.tsv` | 20 | the four doublewords of each of the five table entries at S + 3592 |
| `counts.tsv` | 3 | S + 3784, S + 3792 and S + 3808 |
| `devices.tsv` | 7 | the device object pointers at S + 3816 + 8 * id for ids 0 to 6 |
| `device_fields.tsv` | 20 | +0, +3464, +3472 and +3536 of the device objects of ids 0 to 4 |
| `records.tsv` | 10 | +56 to +80, +1392 and +1408 of the game OS record, and +56 to +80 of partition 1's record |
| `init.tsv` | 2 | S + 3872 and S + 3880 |
| `node.tsv` | 52 | the device and bus nodes: key, the lv1 address of v1, the reads, whether they agree, v1 and v2 |
| `region.tsv` | 84 | the region nodes, in the shape of `node.tsv` |

Each of the first eight files is the output of one `cellgov ps3 peek
--out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x368388,0x7DE40,0x373FB0 --out bus.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EBA0,0x7EBA8,0x7EBB0 --out list.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EBC8,0x7EBD0,...,0x7EC60 --out table.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7EC88,0x7EC90,0x7ECA0 --out counts.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7ECA8,0x7ECB0,...,0x7ECD8 --out devices.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x628010,0x628D98,0x628DA0,0x628DE0,... --out device_fields.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x38B5C8,0x38B5D0,0x38B5D8,0x38B5E0,0x38BB00,0x38BB10,0x6B5738,0x6B5740,0x6B5748,0x6B5750 --out records.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7ECE0,0x7ECE8 --out init.tsv
```
