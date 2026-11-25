# lv2_event_port_ids

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the port ids that lv1 `construct_event_receive_port` (hypercall 18)
gave the kernel on each hardware thread. CellGov's hypervisor answers
each of these calls in a 4.93 kernel boot with the id that this capture
reads for the call's hardware thread and `sc` address.

## Where the values sit

The shipped 4.93 `lv2_kernel.self` calls hypercall 18 four times on each
hardware thread:

- The function that sets up the thread's 640-byte record stores r4 at
  +0x40 of the record [OFW-4.93 lv2_kernel a:0x8000000000007de8]. See
  the README of `lv2_logical_ppe_record` for the record.
- The per-thread start loads the thread index n from +0x00 of the record
  through SPRG0 [OFW-4.93 lv2_kernel a:0x8000000000265668-0x8000000000265670],
  and calls hypercall 18 three times:
  - call A [OFW-4.93 lv2_kernel a:0x80000000002656d0] stores r4 at
    table A + 48 * n + 8
    [OFW-4.93 lv2_kernel a:0x80000000002656dc-0x8000000000265708];
  - call B [OFW-4.93 lv2_kernel a:0x8000000000265784] stores r4 at
    table B + 56 * n [OFW-4.93 lv2_kernel a:0x80000000002657d8];
  - call C [OFW-4.93 lv2_kernel a:0x8000000000265840] stores r4 at
    table C + 40 * n [OFW-4.93 lv2_kernel a:0x8000000000265874].

Each call goes to the kernel's panic path on a nonzero status.

| Address | Stock value | Meaning |
|---|---|---|
| [OFW-4.93 lv2_kernel a:0x8000000000003000] | 0x800000000034fbb0 | r2 |
| [OFW-4.93 lv2_kernel a:0x8000000000347d38] | 0x8000000000368f80 | the TOC slot r2 - 32376: the record array |
| [OFW-4.93 lv2_kernel a:0x8000000000350cf0] | 0x800000000037a668 | the TOC slot r2 + 4416: table A |
| [OFW-4.93 lv2_kernel a:0x8000000000350d20] | 0x80000000003dded0 | the TOC slot r2 + 4464: table B |
| [OFW-4.93 lv2_kernel a:0x8000000000350d48] | 0x80000000004067d0 | the TOC slot r2 + 4504: table C |

The three tables lie in the kernel's writable segment.

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile.
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads the five locator words first and reads the records and the table
  entries only when each holds its stock value. It reads no kernel code
  and writes no kernel memory.

`cgov_frame.bin` holds the 45 reads as (address, doubleword) pairs; the
manifest and `ppu/main.c` give the order.
