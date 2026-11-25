# lv2_logical_ppe_record

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the output of the lv1 `get_logical_ppe_id` call (hypercall 69) that the
kernel made at boot on each hardware thread. CellGov's hypervisor answers
that call in a 4.93 kernel boot with the id that this capture reads.

## Where the values sit

The shipped 4.93 `lv2_kernel.self` sets up one 640-byte record per
hardware thread in the function that holds the `sc 1`
[OFW-4.93 lv2_kernel a:0x8000000000007d2c]. The function:

- loads the record array from the TOC slot at r2 - 32376 and the count
  from the doubleword that the TOC slot at r2 - 32360 points to;
- returns at once when the count is above 1, so it sets up at most two
  records;
- takes the record at array + 640 * count, increments the count, stores
  the old count at +0x00 and puts the record address in SPRG0;
- zeroes +0x48, +0x50 and +0x60, and calls hypercall 69 with `li r11, 69`;
- on status 0 stores r4 at +0x08 [OFW-4.93 lv2_kernel a:0x8000000000007d40],
  and then cntlzw of CTRL at +0x10
  [OFW-4.93 lv2_kernel a:0x8000000000007d58];
- on a nonzero status goes to the kernel's panic routine.

The function then loads +0x08 and +0x10 into r3 and r4 for hypercall 11,
`configure_irq_state_bitmap`
[OFW-4.93 lv2_kernel a:0x8000000000007d68-0x8000000000007d78].

| Address | Stock value | Meaning |
|---|---|---|
| [OFW-4.93 lv2_kernel a:0x8000000000003000] | 0x800000000034fbb0 | r2 |
| [OFW-4.93 lv2_kernel a:0x8000000000347d38] | 0x8000000000368f80 | the TOC slot r2 - 32376: the record array |
| [OFW-4.93 lv2_kernel a:0x8000000000347d48] | 0x8000000000369488 | the TOC slot r2 - 32360: the count |

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile.
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads the three locator words first and reads the count and the
  records only when each holds its stock value. It reads no kernel code
  and writes no kernel memory.

`cgov_frame.bin` holds the 30 reads as (address, doubleword) pairs: the
three locator words, the count, and words 0 to 12 of each record.
