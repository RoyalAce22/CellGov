# lv1_repository_bus

A read-only capture of lpar 1's bus nodes in lv1's repository, on the
reference console at firmware 4.93. The 4.93 kernel reads these nodes at
boot: in its search for the system bus, in its device loop and in its
search for the storage bus.
CellGov's hypervisor answers those reads from this capture, through
`cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1 --width 8` only,
  through the runner's console queue, after the console had booted to
  the XMB, on 2026-11-14. The command reads webMAN's `peek.lv1` pages.
  The capture wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
- The capture walked the whole split-ordered list of lpar 1's
  repository once, from the dummy node of bucket 0 to the list's last
  node. It read the next pointer, the split-order key and the first key
  word of each node. It read the other key words and the values only of
  the nodes whose first key word is `bus` with an index.
- Only the bus nodes of buses 0 to 3 are committed. The raw pages and
  the other nodes are deleted.
- No row of `tests/micro/console_crashes.tsv` covers these addresses.
- No console-identifying value is in this directory.

## The kernel's reads

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these reads. Each one goes through the read
wrapper, which sets r3 = 1 and makes hypercall 91
[OFW-4.93 lv2_kernel a:0x80000000000922c4-0x8000000000092318].

1. The bus search reads `bus.<n>.type` for n = 0 to 3. It stops at the
   first bus whose type is the type that its caller gives
   [OFW-4.93 lv2_kernel a:0x80000000002777f8-0x8000000000277830]. The
   caller gives type 4 [OFW-4.93 lv2_kernel a:0x8000000000277910].
2. For the bus it finds, the search reads `bus.<n>.id` and
   `bus.<n>.num_dev`
   [OFW-4.93 lv2_kernel a:0x8000000000277864-0x80000000002778a8]. The
   caller keeps them in its bus object
   [OFW-4.93 lv2_kernel a:0x800000000027792c-0x8000000000277940].
3. The kernel reads `bus.<n>.id` again and keeps the low word of v2
   when v2 is not 0
   [OFW-4.93 lv2_kernel a:0x8000000000093900-0x8000000000093924].
4. The device loop reads `bus.<n>.dev.<m>.type` for each device, and
   selects a handler for types 3, 4 and 6
   [OFW-4.93 lv2_kernel a:0x80000000000937fc-0x8000000000093844].
5. The handler of a device of type 3 reads `bus.<n>.dev.<m>.id`, and
   gives it to the device object with the bus id
   [OFW-4.93 lv2_kernel a:0x80000000000936cc-0x80000000000936ec].
6. After its open of the device, the handler of a device of type 3 reads
   `bus.<n>.dev.<m>.intr.0`
   [OFW-4.93 lv2_kernel a:0x8000000000093738-0x8000000000093748]. When
   v1 is 2, it keeps v2 at +128 of the device object. Otherwise it logs
   the value and keeps nothing
   [OFW-4.93 lv2_kernel a:0x8000000000093754-0x8000000000093790]. Then
   the device loop reads the type of the next device
   [OFW-4.93 lv2_kernel a:0x80000000000937ec-0x8000000000093808].
7. The handler of a device of type 4 reads `bus.<n>.dev.<m>.id` and
   gives it to its device object. The object then opens the device. The
   handler takes one of two paths
   [OFW-4.93 lv2_kernel a:0x8000000000093448-0x800000000009346c]:
   - for the bus's first device of type 4, the read is at
     [OFW-4.93 lv2_kernel a:0x8000000000093648-0x8000000000093654];
   - for a later device of type 4, the read is at
     [OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490].
8. After the open, the handler of a device of type 4 reads
   `bus.<n>.dev.<m>.reg.<r>.type` and then `.data` for r = 0, 1 and on,
   until a type read fails. Then it reads `bus.<n>.dev.<m>.intr.<i>` for
   i = 0, 1 and on, until a read fails
   [OFW-4.93 lv2_kernel a:0x80000000000934d4-0x80000000000935d4]. Then
   the device loop reads the type of the next device.
9. The handler of a device of type 6 reads `bus.<n>.dev.<m>.id`
   [OFW-4.93 lv2_kernel a:0x8000000000093318-0x8000000000093328] and
   then `bus.<n>.dev.<m>.reg.0.type`. When that read fails, it reads
   `bus.<n>.dev.<m>.intr.0`, and keeps v2 when v1 is 5
   [OFW-4.93 lv2_kernel a:0x8000000000093364-0x80000000000933ac].
10. The storage bus search reads `bus.<n>.id` and then `bus.<n>.type`
    for n = 0 to 9. It stops at the first bus of type 5, and a failed
    read sends it on to the next bus
    [OFW-4.93 lv2_kernel a:0x80000000002871e4-0x8000000000287228]. It
    keeps v1 of the id of the bus that it finds
    [OFW-4.93 lv2_kernel a:0x8000000000287254-0x800000000028725c]. Its id
    accessor drops v2: it gives a slot of its own frame for v2 and frees
    the frame without a load of that slot
    [OFW-4.93 lv2_kernel a:0x80000000000962f8-0x800000000009631c].

The accessor of the `intr` read builds its key from the `bus` and `dev`
words and the word `intr` with an index, and returns v1 and v2
[OFW-4.93 lv2_kernel a:0x8000000000095da8-0x8000000000095ea8]. The
handler's call gives it index 0
[OFW-4.93 lv2_kernel a:0x8000000000095eac-0x8000000000095eb8].
Linux `ps3_repository_read_dev_intr` reads the same key form.

The accessors build the key words from TOC strings: `bus`
[OFW-4.93 lv2_kernel a:0x80000000002d92b8], `dev`
[OFW-4.93 lv2_kernel a:0x80000000002fa1e0], `id`
[OFW-4.93 lv2_kernel a:0x80000000002d66d8], `type`
[OFW-4.93 lv2_kernel a:0x80000000002d92d0], `num_dev`
[OFW-4.93 lv2_kernel a:0x80000000002d92f8], `intr`
[OFW-4.93 lv2_kernel a:0x80000000002d92c8], `reg`
[OFW-4.93 lv2_kernel a:0x80000000002d92c0] and `data`
[OFW-4.93 lv2_kernel a:0x80000000002d92d8]. Linux
`arch/powerpc/platforms/ps3/repository.c` uses the same key layouts.

## Where lv1 keeps the nodes

The README of `tests/micro/lv1_repository_debug_support` gives lv1's
read path, the hash and the node layout. The README of
`tests/micro/lv1_repository_flash` gives the bucket rule. In short:

- The hash is the XOR of the four 32-bit halves of n1 XOR n4 and n2
  XOR n3, with the bits of the mask word cleared
  [OFW-4.93 lv1 a:0x2E3820-0x2E38D8].
- The split-order key is the hash with its 32 bits reversed, OR 1
  [OFW-4.93 lv1 a:0x2E3874-0x2E38D8].
- A node holds its next pointer at +48, its split-order key at +56, the
  four key words at +64, v1 at +104 and v2 at +112.

The walk up to the buckets gave the same values as the walk of
`lv1_repository_flash` on 2026-11-12:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| entry of lpar 1 | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 0 | 0x687BB8 | 0x6881A0, the first dummy node |

The list held 401 nodes in ascending split-order key, and its last node
holds the split-order key 0xFFFFFFFF and a next pointer of 0. Bit 0 of
the next pointer of each served node is clear, so no served node is
deleted.

The served nodes:

| Node | Hash | Bucket | Split-order key | Node address |
|---|---|---|---|---|
| `bus.0.type` | 0x160C0365 | 5 | 0xA6C03069 | 0x689F90 |
| `bus.1.type` | 0x160C0364 | 4 | 0x26C03069 | 0x68AD10 |
| `bus.1.id` | 0x0B117301 | 1 | 0x80CE88D1 | 0x68AC90 |
| `bus.1.num_dev` | 0x2865685E | 14 | 0x7A16A615 | 0x68AD90 |
| `bus.1.dev.0.type` | 0x32697564 | 4 | 0x26AE964D | 0x68A210 |
| `bus.1.dev.0.id` | 0x2F740501 | 1 | 0x80A02EF5 | 0x68A190 |
| `bus.1.dev.0.intr.0` | 0x2F7E7173 | 3 | 0xCE8E7EF5 | 0x68A290 |
| `bus.1.dev.1.type` | 0x32697565 | 5 | 0xA6AE964D | 0x68A390 |
| `bus.1.dev.1.id` | 0x2F740500 | 0 | 0x00A02EF5 | 0x68A310 |
| `bus.1.dev.1.reg.0.type` | 0x000C1265 | 5 | 0xA6483001 | 0x68A510 |
| `bus.1.dev.1.reg.0.data` | 0x10141661 | 1 | 0x86682809 | 0x68A590 |
| `bus.1.dev.1.reg.1.type` | 0x000C1264 | 4 | 0x26483001 | 0x68A610 |
| `bus.1.dev.1.reg.1.data` | 0x10141660 | 0 | 0x06682809 | 0x68A690 |
| `bus.1.dev.1.intr.0` | 0x2F7E7172 | 2 | 0x4E8E7EF5 | 0x68A410 |
| `bus.1.dev.1.intr.1` | 0x2F7E7173 | 3 | 0xCE8E7EF5 | 0x68A490 |
| `bus.1.dev.2.type` | 0x32697566 | 6 | 0x66AE964D | 0x68A790 |
| `bus.1.dev.2.id` | 0x2F740503 | 3 | 0xC0A02EF5 | 0x68A710 |
| `bus.1.dev.2.reg.0.type` | 0x000C1266 | 6 | 0x66483001 | 0x68A910 |
| `bus.1.dev.2.reg.0.data` | 0x10141662 | 2 | 0x46682809 | 0x68A990 |
| `bus.1.dev.2.reg.1.type` | 0x000C1267 | 7 | 0xE6483001 | 0x68AA10 |
| `bus.1.dev.2.reg.1.data` | 0x10141663 | 3 | 0xC6682809 | 0x68AA90 |
| `bus.1.dev.2.intr.0` | 0x2F7E7171 | 1 | 0x8E8E7EF5 | 0x68A810 |
| `bus.1.dev.2.intr.1` | 0x2F7E7170 | 0 | 0x0E8E7EF5 | 0x68A890 |
| `bus.1.dev.3.type` | 0x32697567 | 7 | 0xE6AE964D | 0x68AB90 |
| `bus.1.dev.3.id` | 0x2F740502 | 2 | 0x40A02EF5 | 0x68AB10 |
| `bus.1.dev.3.intr.0` | 0x2F7E7170 | 0 | 0x0E8E7EF5 | 0x68AC10 |

Two pairs of nodes have one hash: `bus.1.dev.1.intr.1` and
`bus.1.dev.0.intr.0`, and `bus.1.dev.3.intr.0` and
`bus.1.dev.2.intr.1`. The walk tells them apart by the four key words
[OFW-4.93 lv1 a:0x2E39D4-0x2E39F8].

## Keys with no node

The walk went over the whole list, and the capture keeps every node of
buses 0 to 3. So a key of those buses that neither file holds names no
node on the reference console. The 4.93 boot reads five such keys:

- `bus.1.dev.1.reg.2.type` and `bus.1.dev.2.reg.2.type`, which end the
  handler's region loop for devices 1 and 2;
- `bus.1.dev.1.intr.2` and `bus.1.dev.2.intr.2`, which end the
  handler's interrupt loop for devices 1 and 2;
- `bus.1.dev.3.reg.0.type`, after which the handler of type 6 goes on
  to interrupt 0.

lv1's lookup returns r3 = 0x0000001EFFFFFFFA when the walk finds no
node [OFW-4.93 lv1 a:0x2E3D38-0x2E3D40]
[OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8]. The handler clears the two values
before the lookup, so r4 and r5 are 0
[OFW-4.93 lv1 a:0x2E52E0-0x2E5300] [OFW-4.93 lv1 a:0x2E53A4-0x2E53B8].
CellGov's hypervisor answers the five keys so, through
`cellgov_ps3_abi::lv1::repository::ABSENT_NODES`.

## What writes the nodes

The lv1 code at [OFW-4.93 lv1 a:0x2520A0-0x2520BC] builds the `bus`
and `dev` key words with immediates, in a loop of up to 7 buses. This
capture does not trace that code. So it does not show how the values
depend on the unit. Its values are those of one CECH-2001A.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 26 | the nodes that the 4.93 boot reads: the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |
| `unread_node.tsv` | 10 | the other bus nodes of buses 0 to 3, in the same columns, for a later boot that reads them |

The rows of `bus.1.dev.0.intr.0` and of the nodes of devices 1 to 3 of
bus 1 moved from `unread_node.tsv` to `node.tsv` when the
4.93 boot came to read them. Their values are those of the one walk on
2026-11-14.

The 4.93 boot's storage bus search also reads `bus.0.id`, `bus.2.id`,
`bus.2.type`, `bus.3.id` and `bus.3.type`. Their rows stay in
`unread_node.tsv`, because the capture files keep the rows as the walk
wrote them. CellGov's hypervisor answers these five reads from those
rows.

## The values

- `bus.0.type` holds 3, so the bus search goes on to bus 1.
- `bus.1.type` holds 4, so bus 1 is the bus that the search finds.
- `bus.1.id` holds v1 = 1 and v2 = 0x04000102.
- `bus.1.num_dev` holds 4.
- `bus.1.dev.0.type` holds 3, so the device loop runs the handler of
  type 3 for device 0.
- `bus.1.dev.0.id` holds 0.
- `bus.1.dev.0.intr.0` holds v1 = 2 and v2 = 0, so the handler of type
  3 keeps 0 at +128 of the device object.
- `bus.1.dev.1.type` holds 4, so the device loop runs the handler of
  type 4 for device 1.
- `bus.1.dev.1.id` holds 1.
- `bus.1.dev.1.reg.0` holds type 3, at 0x3010000 with size 0x10000.
  `bus.1.dev.1.reg.1` holds type 4, at 0x3810000 with size 0x10000.
- `bus.1.dev.1.intr.0` holds v1 = 3 and v2 = 0x10, and
  `bus.1.dev.1.intr.1` holds v1 = 4 and v2 = 0xA.
- `bus.1.dev.2.type` holds 4, so the device loop runs the handler of
  type 4 for device 2. `bus.1.dev.2.id` holds 2.
- `bus.1.dev.2.reg.0` holds type 3, at 0x3020000 with size 0x10000.
  `bus.1.dev.2.reg.1` holds type 4, at 0x3820000 with size 0x10000.
- `bus.1.dev.2.intr.0` holds v1 = 3 and v2 = 0x11, and
  `bus.1.dev.2.intr.1` holds v1 = 4 and v2 = 0xB.
- `bus.1.dev.3.type` holds 6, so the device loop runs the handler of
  type 6 for device 3. `bus.1.dev.3.id` holds 3, and
  `bus.1.dev.3.intr.0` holds v1 = 5 and v2 = 0x2F.
- `bus.0.id` holds 0, `bus.2.id` holds 2 and `bus.3.id` holds 3.
  `bus.2.type` holds 1 and `bus.3.type` holds 2. So no bus of 0 to 3 has
  type 5, and the storage bus search goes on to bus 4. This capture
  keeps no value of bus 4. `tests/micro/lv1_repository_storage_bus`
  gives the bus 4 `id` and `type` values.

The other served values are 0 in v2. lv1 answers each read with r3 = 0
and the node's two values.
