# ppu_msr_le_readback

A hardware probe. It asks whether the PPE keeps MSR[LE] = 1 after an
`rfid` whose SRR1 holds it, or reads the bit back as 0.

[CBE-Handbook p:739 s:A.2.5] says that a write to MSR[LE] or MSR[ILE]
has no effect. It does not say what a read returns. A problem-state
program cannot run `mtmsrd` or `rfid`, so the probe makes the 4.93
kernel's own system-call return do the write.

## Where the values sit

The kernel's system-call vector moves the caller's SRR0 and SRR1 to r3
and r4 and goes to its handler
[OFW-4.93 lv2_kernel a:0x8000000000000c2c-0x8000000000000c5c]. The
handler finds the current thread through SPRG0 (record + 0x68, then the
thread's kernel stack base at +0x50 and size at +0x58), puts a frame
288 bytes below the stack top, and stores the caller's SRR0 at
frame + 0x10 and SRR1 at frame + 0x18
[OFW-4.93 lv2_kernel a:0x8000000000275a90-0x8000000000275abc]. The fast
exit loads frame + 0x18, inserts MSR bits 38 and 50 from the thread,
and returns by `rfid`
[OFW-4.93 lv2_kernel a:0x8000000000275cf8-0x8000000000275d38]. The slow
exit copies frame + 0x18 to the thread for the later return
[OFW-4.93 lv2_kernel a:0x8000000000275df0-0x8000000000275dfc].

## Method

- Firmware 4.93, the `cech20-cex-493` reference profile.
- The probe reads the same three stock locators as
  `lv2_logical_ppe_record`, follows each hardware-thread record to the
  current thread's frame, and keeps the frame whose saved SRR0 is the
  address after the probe's own peek `sc`.
- During the CFW's lv2 poke (syscall 7) it writes S0 | LE | PMM to its
  own frame + 0x18, so the poke's `rfid` restores both bits. PMM is a bit
  the PPE keeps, so a set PMM at the next `sc` shows that the written
  SRR1 reached the MSR. A second poke clears both bits again.
- Every address it reads or writes is a kernel record, thread or stack
  slot that the kernel itself stores to.

`cgov_frame.bin` holds 24 doublewords; `manifest.toml` names each.
