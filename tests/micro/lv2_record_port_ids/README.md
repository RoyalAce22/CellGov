# lv2_record_port_ids

A read-only capture of the 4.93 kernel's two per-thread records on the
reference console. It gives the port id that lv1's
`construct_event_receive_port` (hypercall 18) gave the kernel's call at
[OFW-4.93 lv2_kernel a:0x8000000000027d20] on each hardware thread. The
kernel keeps that id at +0x38 of the thread's record.

CellGov's hypervisor answers that call with these ids through
`GAME_OS_PORT_CALLS` in `cellgov_ps3_abi::lv1::event_port`. The test
`event_port_record_console_tests` holds each row to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the read returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-12, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke and
  installed nothing on the console.
- The words are in the kernel's writable data segment, and no row of
  `tests/micro/console_crashes.tsv` covers them.
- Only the eight words below are committed.
- No console-identifying value is in this directory.
- The probe frame of `lv2_logical_ppe_record`, captured on 2026-11-11,
  holds words 0 to 12 of each record. Its words at +0x00, +0x10 and
  +0x38 of each record agree with this capture.

## Where the records are

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives the addresses. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

- The TOC slot r2 - 32376 [OFW-4.93 lv2_kernel a:0x8000000000347d38]
  holds 0x8000000000368f80, the array of 640-byte records. See the
  README of `lv2_logical_ppe_record`.
- The record setup stores the record's index at +0x00, and puts the
  record's address in SPRG0
  [OFW-4.93 lv2_kernel a:0x8000000000007c08]. It stores cntlzw of CTRL at
  +0x10 [OFW-4.93 lv2_kernel a:0x8000000000007d58], so +0x10 is the
  hardware thread that CTRL names.

## Where the kernel stores the id

- The function at 0x8000000000027c2c takes the record in r4 and keeps it
  in r31 [OFW-4.93 lv2_kernel a:0x8000000000027c58].
- It returns -1 at once when +0x48 of the record is not 0
  [OFW-4.93 lv2_kernel a:0x8000000000027c3c-0x8000000000027c64].
- Otherwise it stores an address at +0x48
  [OFW-4.93 lv2_kernel a:0x8000000000027d08], and sends hypercall 18
  [OFW-4.93 lv2_kernel a:0x8000000000027d1c-0x8000000000027d20].
- On r3 = 0 it stores r4 at +0x38 of the record
  [OFW-4.93 lv2_kernel a:0x8000000000027d44]. It then gives that id in
  r3 and the record's index in r4 to the kernel's connect step
  [OFW-4.93 lv2_kernel a:0x8000000000027d30-0x8000000000027d4c], which
  sends `connect_irq_plug_ext` (hypercall 12).

## Which thread calls it

The per-thread start loads the record's index through SPRG0 into r31
[OFW-4.93 lv2_kernel a:0x8000000000265668-0x8000000000265670] and
compares it with 0 in cr3
[OFW-4.93 lv2_kernel a:0x80000000002656ac]. Both calls of the function
give it the record from SPRG0
[OFW-4.93 lv2_kernel a:0x8000000000265920-0x8000000000265934].

- The thread with index 1 reaches the two calls directly: the branch on
  cr3 is not taken [OFW-4.93 lv2_kernel a:0x80000000002658d8].
- The thread with index 0 takes that branch. It first calls the setup of
  the kernel's boot logical SPE
  [OFW-4.93 lv2_kernel a:0x8000000000265960], and then branches back to
  the calls [OFW-4.93 lv2_kernel a:0x80000000002659c0].
So each thread makes this call with its own record, and the record's
+0x38 holds the id of that thread's call.

## Files

| File | Rows | Holds |
|---|---|---|
| `record_ports.tsv` | 8 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are `toc` (the TOC word), `records` (the TOC slot of the
record array), and for each record n: `index_n` (+0x00), `ctrl_n`
(+0x10) and `port_n` (+0x38).

## The live records

- The TOC word and the TOC slot hold their stock values, so the records
  are at the addresses that the image gives.
- Record 0 holds index 0 and names hardware thread 0. Record 1 holds
  index 1 and names hardware thread 1.
- The port id of hardware thread 0 is 17 (0x11). The port id of hardware
  thread 1 is 12 (0x0c). Neither is 0, so each call returned r3 = 0.
- lv1 gives a new port the first free id of the partition's object
  table; see `cellgov_ps3_abi::lv1::event_port`. These two ids fit that
  order:
  - Thread 0 makes its call after the boot SPE's port, which took 16
    (capture `lv2_logical_spe_object`), so its port took 17.
  - Thread 1 makes its call after its own start calls, which took 5 to
    7 (capture `lv2_event_port_ids`). Its port took 12, so ids 8 to 11
    were in use at the time of the call. Thread 0's start calls took 8
    to 10 (capture `lv2_event_port_ids`), and the boot SPE object took
    11 (capture `lv2_logical_spe_object`).
- On thread 0, one `construct_logical_spe` (hypercall 57) gives the SPE
  object its id and then makes the SPE's three outlets, which took 13 to
  15; see `cellgov_ps3_abi::lv1::spe`. If no object released its id in
  that time, thread 1 made its call while thread 0 was in that
  hypercall, after the SPE object took its id and before the first
  outlet took its id. So the two hardware threads were in lv1 at the
  same time. The capture `lv2_logical_spe_object` found no read for id
  12; this capture names it.
