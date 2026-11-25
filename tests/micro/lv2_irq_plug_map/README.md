# lv2_irq_plug_map

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the map of the IRQ plugs that the kernel connected on each hardware
thread with the lv1 call `connect_irq_plug_ext` (hypercall 12), and the
interrupt state bitmap of the thread in the same pass.

## Where the values sit

The records and their locators are the ones that `lv2_logical_ppe_record`
reads.

The shipped 4.93 `lv2_kernel.self` picks the plug of each call in the
function at [OFW-4.93 lv2_kernel a:0x8000000000007ab4]. It loads each of
the four doublewords at +0xE0 of the record in turn, takes the first
clear bit with `cntlzd`, most significant bit first, and sets that bit
[OFW-4.93 lv2_kernel a:0x8000000000007ae0-0x8000000000007b08]. The plug
is 64 times the doubleword's index plus the bit's count of leading ones.
The kernel's connect step passes that plug in r5 of hypercall 12
[OFW-4.93 lv2_kernel a:0x80000000000087f8].

The bitmap at +0x1C0 is the one that `lv2_irq_state_bitmaps` reads: four
status doublewords, then four mask doublewords at +32. lv1 clears the
status bit of a plug when it connects a port with no pending event to
it.

The values this probe reads are the state after boot.

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile.
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads the three locator words first and reads the rest only when each
  holds its stock value. It reads no kernel code and writes no kernel
  memory.

`cgov_frame.bin` holds the 30 reads as (address, doubleword) pairs: the
three locator words, the count, and for each record the word at +0x10,
the four map doublewords from +0xE0 and the eight bitmap doublewords
from +0x1C0.
