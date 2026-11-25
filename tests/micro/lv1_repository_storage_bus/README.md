# lv1_repository_storage_bus

A read-only capture of lpar 1's `bus.4.id` and `bus.4.type` nodes in
lv1's repository, on the reference console at firmware 4.93. The 4.93
kernel reads these two nodes at boot, in its search for the storage
bus. CellGov's hypervisor answers those reads from this capture,
through `cellgov_ps3_abi::lv1::repository`.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the values the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A,
  a retail (CEX) unit, on 4.93 with custom firmware (EvilNAT 4.93 PEX,
  Cobra 8.5) and webMAN MOD 1.47.48. It is not a debug or tool unit.
- The capture used `cellgov ps3 peek --space lv1 --width 8` only,
  through the runner's console queue, after the console had booted to
  the XMB, on 2026-11-17. The command reads webMAN's `peek.lv1` pages.
  The capture wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The command read each page twice. The two reads of every word agree.
- The capture did not walk the list again. The walk of
  `tests/micro/lv1_repository_list` gives the address of each node. For
  each of the two nodes, the capture read the next pointer, the
  split-order key, the four key words, v1 and v2.
- `node.tsv` keeps the two values of each node only. The raw pages are
  deleted.
- No row of `tests/micro/console_crashes.tsv` covers these addresses.
- No console-identifying value is in this directory.

## The kernel's reads

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these reads. Each one goes through the read
wrapper, which sets r3 = 1 and makes hypercall 91
[OFW-4.93 lv2_kernel a:0x80000000000922c4-0x8000000000092318].

1. The storage bus search reads `bus.<n>.id` and then `bus.<n>.type`
   for n = 0 to 9. It stops at the first bus of type 5, and a failed
   read sends it on to the next bus
   [OFW-4.93 lv2_kernel a:0x80000000002871e4-0x8000000000287228].
   `tests/micro/lv1_repository_bus` gives buses 0 to 3. None of them
   has type 5.
2. For the bus that it finds, the search gives v1 of the bus id to the
   function at 0x80000000000ADC28
   [OFW-4.93 lv2_kernel a:0x8000000000287254-0x800000000028725c]. That
   function keeps the id at +8 of the search object
   [OFW-4.93 lv2_kernel a:0x80000000000adc58].
3. Then the search calls the function at 0x80000000000AE0C0
   [OFW-4.93 lv2_kernel a:0x8000000000287264-0x800000000028726c]. That
   function makes hypercall 170 with r3 = the kept bus id, r4 = -1 and
   r5 = 0 [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae110].
   No repository read comes between the bus 4 type read and that
   hypercall.

## Where lv1 keeps the nodes

The README of `tests/micro/lv1_repository_debug_support` gives lv1's
read path, the hash and the node layout. In short, a node holds its
next pointer at +48, its split-order key in the high word at +56, the
four key words at +64, v1 at +104 and v2 at +112.

The walk up to the buckets gave the same values as the walks of
`lv1_repository_bus` and `lv1_repository_list`:

| Step | Address | Read |
|---|---|---|
| mask word | 0x373D08 | high word 0xC0000000 |
| partition table pointer | 0x373D18 | 0x609E00 |
| entry of lpar 1 | 0x60F278 | 0x66FA30 |
| lpar 1 partition + 56 | 0x66FA68 | 0x687060, the repository |
| table + 24, n | 0x687080 | high word 1, so 16 buckets |
| table + 32, segment 0 | 0x687088 | 0x687B80 |
| bucket 0 | 0x687BB8 | 0x6881A0, the first dummy node |

The two nodes:

| Node | Node address | Split-order key | Next pointer |
|---|---|---|---|
| `bus.4.id` | 0x694210 | 0x20CE88D1 | 0x68F210 |
| `bus.4.type` | 0x694290 | 0x86C03069 | 0x68C590 |

Each node's four key words are its key, and each split-order key is the
one that `list.tsv` of `lv1_repository_list` gives for the node. Bit 0
of each next pointer is clear, so neither node is deleted.

## Files

| File | Rows | Holds |
|---|---|---|
| `node.tsv` | 2 | the two nodes: the firmware, the key, the lv1 address of v1, the number of reads, whether they agree, and v1 and v2 |

## The values

- `bus.4.id` holds v1 = 4 and v2 = 0.
- `bus.4.type` holds v1 = 5 and v2 = 0. So bus 4 is the first bus of
  type 5, and the storage bus search stops at bus 4. It reads no node
  of buses 5 to 9.

lv1 answers each read with r3 = 0 and the node's two values
[OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
