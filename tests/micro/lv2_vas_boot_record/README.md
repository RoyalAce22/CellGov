# lv2_vas_boot_record

A hardware probe. It reads back, from the running 4.93 kernel's memory,
the outputs of the lv1 `construct_virtual_address_space` call (hypercall
2) that the kernel made at boot. CellGov's hypervisor answers that call
in a 4.93 kernel boot from this capture. No public source gives the vas
id that lv1 returns.

## Where the values sit

The shipped 4.93 `lv2_kernel.self` constructs its virtual address space
in the function at [OFW-4.93 lv2_kernel a:0x800000000005dc98]. Its only
caller, at [OFW-4.93 lv2_kernel a:0x800000000005dd54], passes the record
address that it loads from the TOC slot at r2 - 25760. The function
stores these words in the record:

| Offset | Value | How the kernel gets it |
|---|---|---|
| +0x30 | the vas id | r4 out of hypercall 2 |
| +0x40 | 0x800000000f000000 | a constant the function stores |
| +0x48 | the hash table size | r5 out of hypercall 2 |
| +0x50 | the hash table size >> 7 | derived from r5 |

Before the `sc` at [OFW-4.93 lv2_kernel a:0x800000000005dcf4], the
function stores 0 to +0x30,
+0x38, +0x48 and +0x50. It calls hypercall 2 with r3 = 0, r4 = 2 and
r5 = 0x1014000000000000, which names the page shifts 16 and 20. On
status 0 it stores r4 and r5, and then calls hypercall 7,
`select_virtual_address_space`, with the vas id in r3. A nonzero status
from either call goes to the kernel's panic routine.

The record is at 0x80000000003e3a80. The kernel loads r2 from the
doubleword at [OFW-4.93 lv2_kernel a:0x8000000000003000].

These facts make the record words the outputs of the boot call:

- In the kernel's text, the only load of the record's TOC slot is at
  [OFW-4.93 lv2_kernel a:0x800000000005dd60], in the caller.
- The only `sc 1` that follows a `li r11, 2` within six instructions is
  the one at [OFW-4.93 lv2_kernel a:0x800000000005dcf4].
- The caller also stores the record address in the pointer at TOC slot
  r2 - 25896, so other code can reach the record. That analysis does not
  show which words that code writes. The word at +0x38 is 0 after the
  function returns, and the capture reads 0x100 there.

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile; captured
  2026-11-10 (the provenance records the time).
- The probe reads each word through the CFW's lv2 peek, syscall 6. It
  reads no kernel code and writes no kernel memory.

## The values

| Address | Doubleword | Meaning |
|---|---|---|
| 0x8000000000003000 | 0x800000000034fbb0 | r2, as in the stock kernel |
| 0x8000000000349710 | 0x80000000003e3a80 | the TOC slot: the record, as in the stock kernel |
| 0x80000000003e3ab0 | 0x0000000000000002 | the vas id = 2 |
| 0x80000000003e3ac0 | 0x800000000f000000 | the function's constant |
| 0x80000000003e3ac8 | 0x0000000000040000 | the hash table size = 2^18 bytes |
| 0x80000000003e3ad0 | 0x0000000000000800 | 0x40000 >> 7 |

`cgov_frame.bin` holds all 14 reads as (address, doubleword) pairs: the
two locator words and the record's first 12 doublewords.

## Cross-checks

- The constant at +0x40 is the value that the stock function stores
  [OFW-4.93 lv2_kernel a:0x800000000005dccc], so the probe read the
  record that the function wrote.
- The word at +0x50 is the word at +0x48 shifted right by 7, as the
  function computes it.
- The hash table size agrees with the lv1 handler of hypercall 2
  [OFW-4.93 lv1 a:0x2EC554].
  The handler raises an exponent below 18 to 18, and returns 2 to the
  power of the exponent in r5 (`cellgov_ps3_abi::lv1::address_space`).
  The kernel passes 0.
