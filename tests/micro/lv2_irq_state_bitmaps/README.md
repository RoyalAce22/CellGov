# lv2_irq_state_bitmaps

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the interrupt state bitmap that the kernel registered on each hardware
thread with the lv1 call `configure_irq_state_bitmap` (hypercall 11) at
boot.

## Where the values sit

The records and their locators are the ones that `lv2_logical_ppe_record`
reads. In the function that holds the `sc 1`
[OFW-4.93 lv2_kernel a:0x8000000000007d78], the shipped 4.93
`lv2_kernel.self`:

- zeroes the 64 bytes at +0x1C0 of the record
  [OFW-4.93 lv2_kernel a:0x8000000000007c98-0x8000000000007cc4];
- passes record + 0x1C0 to the function at
  [OFW-4.93 lv2_kernel a:0x8000000000077760], which returns the address
  that the kernel loads into r5 of hypercall 11
  [OFW-4.93 lv2_kernel a:0x8000000000007d3c-0x8000000000007d74];
- loads +0x08 into r3 and +0x10 into r4.

The kernel's linear map puts the bitmap of record 0 at the partition
address 0x369140 and the bitmap of record 1 at 0x3693C0.

A bitmap is four status doublewords, then four mask doublewords at +32.
lv1 sets a status bit for an interrupt on a plug connected to the
thread. The status words this probe reads are the state after boot,
when the kernel has connected its plugs.

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile.
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads the three locator words first and reads the rest only when each
  holds its stock value. It reads no kernel code and writes no kernel
  memory.

`cgov_frame.bin` holds the 22 reads as (address, doubleword) pairs: the
three locator words, the count, and for each record the word at +0x10
and the eight bitmap doublewords from +0x1C0.
