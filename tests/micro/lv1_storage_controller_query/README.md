# lv1_storage_controller_query

A read-only capture, in lv1 on the reference console, of the lv1 state
that decides lv1's answer to the 4.93 kernel's
`storage_send_device_command` (hypercall 248) to storage device 2 with
command 0x1A, the controller query
[OFW-4.93 lv2_kernel a:0x80000000000ab390]. The capture gives three
parts of that state:

- the policy that lets the game OS partition send the command;
- the command ring and the controller of device 2;
- the request tags of device 2.

CellGov's hypervisor answers the query through
`cellgov_ps3_abi::lv1::storage_command`. The test
`storage_command_console_tests` holds the answer's assumptions to this
capture.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words that the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-23, `cellgov ps3 peek --space lv1` read the words below
  through webMAN's `peek.lv1` command, after the console had booted to
  the XMB. Each read took the address that the read before it gave. It
  read each page twice, and the two reads of every word agree. It wrote
  nothing to lv1 or lv2, used no poke, sent no hypercall and installed
  nothing on the console.
- Every address is lv1 heap or lv1 data. No address is MMIO. No row of
  `tests/micro/console_crashes.tsv` covers these addresses: each row is
  an lv2 range. No page that a read touched covers the body of the
  region access check's ACL comparison
  [OFW-4.93 lv1 a:0x25B6F0-0x25B7C8].
- The walk of `tests/micro/lv1_repository_list` gave 0xE4E20 as the
  node of `ss.laid` for lpar 2. At this boot that address held another
  node. So the capture walked the node's bucket again, and it took the
  device object from the storage bus object again. The first read of
  0xE4E20 is not committed.
- The capture read the repository walk one step at a time. Then it read
  the same addresses again in the four commands below. The two reads
  agree. Only the second reads are committed.
- A field that lv1 reads as a 32-bit word was read with `--width 4`.
  The other half of its doubleword is not committed.
- No console-identifying value is in this directory. The `ss.laid`
  value is the authority id of the game OS partition, which is the same
  on each console.

## lv1's path

The shipped 4.93 `lv1.self`, decrypted offline and read with
`cellgov dev disasm`, gives the path. D is the object of device 2.

The command dispatcher 0x272BE4 does these steps for the query:

1. For mode 0 it asks the policy check with r3 = the partition, r4 = 1,
   r5 = the command id shifted left 32 and r6 = 1
   [OFW-4.93 lv1 a:0x272F84-0x272FAC]. A refusal gives -4
   [OFW-4.93 lv1 a:0x272FB0-0x272FB8].
2. While the slot of the ring index is ready and the controller is
   idle, it starts the command [OFW-4.93 lv1 a:0x272D20-0x272D44]
   [OFW-4.93 lv1 a:0x272E20-0x272E80]. Otherwise it queues the command
   [OFW-4.93 lv1 a:0x272D48-0x272DCC].

### The policy

- The check 0x31D7B0 loads G from the TOC slot r2 + 120, loads the table
  T from G and moves its arguments up by one register before it
  branches to 0x31D34C [OFW-4.93 lv1 a:0x31D7B0-0x31D7D4]. The TOC slot
  is 0x35A0B0.
- An index outside 1 to 4 gives -17 [OFW-4.93 lv1 a:0x31D35C-0x31D390].
  The check takes the lock word at T + 56
  [OFW-4.93 lv1 a:0x31D394-0x31D3B8].
- When the doubleword at T + 0 is 0, only partition 1 passes, and every
  other partition gets -4 [OFW-4.93 lv1 a:0x31D3BC-0x31D3E4].
- Otherwise the check calls the key function with the partition
  [OFW-4.93 lv1 a:0x31D418-0x31D430]. The function looks up the key
  (`ss`, `laid`, the partition, 0) in lpar 1's repository
  [OFW-4.93 lv1 a:0x31CE9C-0x31CEF8]. For a node that it finds, the key
  ID is the node's v1 [OFW-4.93 lv1 a:0x31CEFC-0x31CF0C]
  [OFW-4.93 lv1 a:0x2E4AB4] [OFW-4.93 lv1 a:0x2E3E2C-0x2E3E34].
- The entry pointer H is the doubleword at T + 16 + 8 * index. For
  index 1 it is T + 24 [OFW-4.93 lv1 a:0x31D434-0x31D444].
- The doubleword at H + 8 is the byte count of the entry's records. The
  records are 32 bytes each and start at H + 16
  [OFW-4.93 lv1 a:0x31D448-0x31D4B0].
- A record holds X at +0, M at +8, A at +16 and V at +24
  [OFW-4.93 lv1 a:0x31D4B8-0x31D50C] [OFW-4.93 lv1 a:0x31D518-0x31D5C8]
  [OFW-4.93 lv1 a:0x31D6E8].
- The check walks the records in order. A record with A = 0 or A = the
  command decides when (X AND M) = (ID AND M)
  [OFW-4.93 lv1 a:0x31D5C8-0x31D5DC] [OFW-4.93 lv1 a:0x31D6D8-0x31D6E4].
  Other records do not decide [OFW-4.93 lv1 a:0x31D5E0-0x31D5E8].
- The deciding record passes the command (0) when V AND 1 = 1, with the
  1 of the dispatcher's r6. Otherwise it gives -4
  [OFW-4.93 lv1 a:0x31D744-0x31D764]. When no record decides, the check
  gives -4 [OFW-4.93 lv1 a:0x31D768-0x31D778].

### The ring and the controller

- The controller pointer is the doubleword at D + 5688. The controller
  word is the word at +16 of the controller
  [OFW-4.93 lv1 a:0x272D20-0x272D28].
- The ring index is the word at D + 7688. The slot is at
  D + 6272 + 88 * index, and its state is the word at +8 of the slot
  [OFW-4.93 lv1 a:0x272D2C-0x272D40].
- A slot state of 1 sends the dispatcher to the start test
  [OFW-4.93 lv1 a:0x272D40-0x272D44]. The start needs a controller word
  of 1 [OFW-4.93 lv1 a:0x272E20-0x272E24]. The start writes state 2 to
  the slot [OFW-4.93 lv1 a:0x272E5C].
- Otherwise the dispatcher scans the 16 slots from the index for a slot
  of state 1 [OFW-4.93 lv1 a:0x272D64-0x272DA0].
- The controller's query function returns the busy status while the
  controller word is 0 [OFW-4.93 lv1 a:0x25F8B0-0x25F8C8]. Otherwise it
  stores 0 in the word [OFW-4.93 lv1 a:0x25F910-0x25F918].

### The request tags

- The request method calls the request function under the lock at
  D + 2672 [OFW-4.93 lv1 a:0x256CD4-0x256D2C]
  [OFW-4.93 lv1 a:0x256D54-0x256D6C].
- The request function reads the tag bitmap, the doubleword at
  D + 2680. When the 16 tag bits are all set it returns -9. Otherwise
  it takes the lowest clear bit as the tag
  [OFW-4.93 lv1 a:0x25398C-0x2539CC] [OFW-4.93 lv1 a:0x2539DC-0x253A04].
- The request method passes no doubleword of D + 3464 to the request
  function [OFW-4.93 lv1 a:0x256D54-0x256D6C]. On the device object,
  D + 3464 is the block size that the registration writes to
  `blk_size` (see `lv1_storage_registration`). The storage bus object
  keeps the word that its own requests take at the same offset (see
  `lv1_storage_notify_request`).

## The values

| Read | Address | Value | What it gives |
|---|---|---|---|
| TOC slot r2 + 120 | 0x35A0B0 | 0x374210 | G |
| G | 0x374210 | 0x61CB00 | the policy table T |
| T + 0 | 0x61CB00 | 0x15D000 | not 0, so the records decide |
| T + 8 to T + 48 | 0x61CB08 | 0x570, 0, 0x15D010, 0x15D420, 0x15D4B0, 0x15D520 | the table's other doublewords and the four entry pointers |
| T + 24, entry 1 | 0x61CB18 | 0x15D010 | H |
| T + 56, lock | 0x61CB38 | word 0 | the lock is free |
| H + 0, H + 8 | 0x15D010 | 1, 0x400 | 32 records |
| record 0 | 0x15D020 | X 0x1010000000000000, M 0xF010000000000000, A 0, V 1 | the record that decides for the game OS partition |
| records 1 to 30 | 0x15D040 | see `records.tsv` | records of other keys |
| record 31 | 0x15D400 | X 0, M 0, A 0, V 0 | a last record that refuses each key |
| `ss.laid` of lpar 2 | 0x92790 | key (`ss`, `laid`, 2, 0), v1 0x1070000002000001, v2 0 | ID |
| S | 0x368388 | 0x7DDC0 | the storage bus object |
| S + 3816 + 8 * 2 | 0x7ECB8 | 0x660010 | D |
| D + 0 | 0x660010 | 0x35E370 | the method table of device 2 |
| D + 2680 | 0x660A88 | 0 | no tag held |
| D + 3464 | 0x660D98 | 0x800 | the block size of device 2 |
| D + 5688 | 0x661648 | 0x7F850 | the controller |
| controller + 16 | 0x7F860 | word 1 | the controller is idle |
| D + 7688 | 0x661E18 | word 0 | the ring index |
| D + 6272 + 88 * i + 8 | 0x661898 | word 1 for each i from 0 to 15 | each slot is ready |

- ID AND M is 0x1010000000000000, which is X AND M of record 0. Record
  0 has A = 0 and V = 1. So the check passes each command of the game
  OS partition, the controller query too.
- The slot of the ring index holds state 1, and the controller word is
  1. So the dispatcher starts the query in the call.
- The tag bitmap is 0. So the query gets tag 0, the lowest free tag.

The read cannot show the state at the time of the kernel's first query.
The values fit a device that holds no command and no request between
two commands.

## The repository walk

The bucket rule is in `tests/micro/lv1_repository_flash`. The key
(`ss`, `laid`, 2, 0) hashes to 0x1F126966, so it is in bucket 6, and its
split-order key is 0x669648F9.

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| partition table + 24, the segment list | 0x609E18 | 0x60F140 |
| segment 0 | 0x60F140 | 0x609F60 |
| segment 0 + 48, the entry size | 0x609F90 | 8 |
| segment 0 + 72, its entry array | 0x609FA8 | 0x60F270 |
| entry of lpar 1 | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 6 | 0x687BE8 | 0x687F60, a dummy node with split-order key 0x60000000 |
| four nodes with lower split-order keys | | 0x68DA90, 0x763E70, 0x68C910, 0x68A910 |
| `ss.laid` node of lpar 2 | 0x92750 | split-order key 0x669648F9 |

## Files

| File | Rows | Holds |
|---|---|---|
| `pointer.tsv` | 2 | the TOC slot and G |
| `table.tsv` | 7 | T + 0 to T + 48 |
| `lock.tsv` | 1 | the word at T + 56 |
| `entry.tsv` | 2 | H + 0 and H + 8 |
| `records.tsv` | 128 | the four doublewords of each of the 32 records |
| `repository.tsv` | 9 | the doublewords of the walk to bucket 6 |
| `walk_next.tsv` | 6 | the next pointer at +48 of the dummy node and of the five nodes |
| `walk_words.tsv` | 8 | the mask word, n, and the split-order key at +56 of the dummy node and of the five nodes |
| `laid.tsv` | 6 | the four key words at +64, v1 at +104 and v2 at +112 of the `ss.laid` node |
| `bus.tsv` | 1 | the storage bus object pointer |
| `device.tsv` | 1 | the object pointer of device 2 |
| `object.tsv` | 4 | D + 0, D + 2680, D + 3464 and D + 5688 |
| `controller.tsv` | 1 | the word at +16 of the controller |
| `ring.tsv` | 17 | the ring index and the state word of each of the 16 slots |

Each file is the output of one `cellgov ps3 peek --out` command:

```text
cellgov ps3 peek --space lv1 --width 8 --address 0x35A0B0,0x374210 --out pointer.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x61CB00,0x61CB08,0x61CB10,0x61CB18,0x61CB20,0x61CB28,0x61CB30 --out table.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x61CB38 --out lock.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x15D010,0x15D018 --out entry.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x15D020,0x15D028,...,0x15D418 --out records.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x373D18,0x609E18,0x60F140,0x609F90,0x609FA8,0x60F278,0x66FA68,0x687088,0x687BE8 --out repository.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x687F90,0x68DAC0,0x763EA0,0x68C940,0x68A940,0x92780 --out walk_next.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x373D08,0x687080,0x687F98,0x68DAC8,0x763EA8,0x68C948,0x68A948,0x92788 --out walk_words.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x92790,0x92798,0x927A0,0x927A8,0x927B8,0x927C0 --out laid.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x368388 --out bus.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x7ECB8 --out device.tsv
cellgov ps3 peek --space lv1 --width 8 --address 0x660010,0x660A88,0x660D98,0x661648 --out object.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x7F860 --out controller.tsv
cellgov ps3 peek --space lv1 --width 4 --address 0x661E18,0x661898,0x6618F0,0x661948,0x6619A0,0x6619F8,0x661A50,0x661AA8,0x661B00,0x661B58,0x661BB0,0x661C08,0x661C60,0x661CB8,0x661D10,0x661D68,0x661DC0 --out ring.tsv
```

`records.tsv` reads each doubleword from 0x15D020 to 0x15D418, in steps
of 8.
