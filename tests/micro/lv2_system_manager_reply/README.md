# lv2_system_manager_reply

A read-only capture from the 4.93 kernel's memory on the reference
console. It gives the fields that the kernel kept from lv1's system
manager reply of service id 0x0B. That reply answers the kernel's
request of service id 9 with payload byte 1 = 2, which the kernel sends
on virtual UART port 2 at boot.

Two of the reply's inputs are bytes of the system manager's bss that it
sets while it runs. The word at reply bytes 4 to 7 is a field of a
system controller reply. So no image read gives the reply. The kernel
keeps the reply's fields in its system manager object, and this capture
reads them. The test `uart_system_manager_reply_console_tests` holds
`cellgov_ps3_abi::lv1::uart::GAME_OS_SYSTEM_MANAGER_REPLIES` to this
capture.

A second read gives the bytes that the kernel kept from lv1's reply of
service id 0x0A. That reply answers the kernel's request of service id
9 with payload byte 1 = 1. See "The stored bytes" below.

No probe runs, so this directory has no manifest and no runner frame.
The files under `peek/cech20-cex-493/` hold the words the reads
returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-14, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The kernel stores to each field of the object that this capture reads,
  so its page is writable
  [OFW-4.93 lv2_kernel a:0x8000000000261a94-0x8000000000261ab8]. The
  object is in the zero-fill part of the kernel's data segment, which
  also holds the records that the capture `lv2_record_port_ids` read.
- No row of `tests/micro/console_crashes.tsv` covers these words.
- No console-identifying value is in this directory.

## Where the kernel keeps the reply

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps. The kernel's r2 is
0x800000000034fbb0 [OFW-4.93 lv2_kernel a:0x8000000000003000].

1. The boot loads the object's address from the TOC slot r2 + 4336, and
   gives it to the request function twice: first with r4 = 2, then with
   r4 = 1 [OFW-4.93 lv2_kernel a:0x8000000000265518-0x8000000000265530].
   That slot holds 0x8000000000474c38
   [OFW-4.93 lv2_kernel a:0x8000000000350ca0-0x8000000000350ca4]. The
   TOC slot r2 + 3864 holds the same address
   [OFW-4.93 lv2_kernel a:0x8000000000350ac8-0x8000000000350acc].
2. The request function sends a request of service id 9 with its r4 in
   payload byte 1 [OFW-4.93 lv2_kernel a:0x80000000002646d8-0x800000000026471c].
   The send function writes the request through +8 of the object
   [OFW-4.93 lv2_kernel a:0x8000000000261ea0-0x8000000000261ea4].
3. The receive loop reads each message through +8 of its argument
   [OFW-4.93 lv2_kernel a:0x8000000000264b84]
   [OFW-4.93 lv2_kernel a:0x8000000000264bc0]. It takes the service id
   at +8 of the header and selects its case through the table at
   0x8000000000264c70, with index service id - 3
   [OFW-4.93 lv2_kernel a:0x8000000000264c40-0x8000000000264c6c]
   [OFW-4.93 lv2_kernel a:0x8000000000350bd0-0x8000000000350bd4].
4. Entry 8 of the table, for service id 0x0B, is 0x13c, so the case is
   at 0x8000000000264dac. That case calls the function at
   0x8000000000261a94 with the object and the payload
   [OFW-4.93 lv2_kernel a:0x8000000000264dac-0x8000000000264db4].
5. That function stores the reply's fields in the object
   [OFW-4.93 lv2_kernel a:0x8000000000261a94-0x8000000000261ab8]:
   - payload byte 1 at +2032 and payload byte 2 at +2033;
   - the word at payload byte 4 at +2048;
   - the doubleword at payload byte 8 at +2040.
   It keeps no other payload byte.

So the object's +2032 to +2051 hold reply bytes 1, 2 and 4 to 15. lv1's
reply function sets byte 0 to 2 and byte 3 to 0
[OFW-4.93 lv1 a:0x44BAF4-0x44BB14], so the capture gives all 16 bytes.

## Why the kept fields are the boot's fields

- lv1 sends service id 0x0B at one place only: the case of payload byte
  1 = 2 of the handler of service id 9 [OFW-4.93 lv1 a:0x44DF18-0x44DF24].
- The kernel's boot sends that request once, before its request with
  payload byte 1 = 1. The second request gets a reply of service id
  0x0A, which the kernel does not keep in these fields.
- The word at reply bytes 4 to 7 is the word at the system manager's
  vaddr 0xC000D2D8. The only store to it is in the system manager's
  system controller setup [OFW-4.93 lv1 a:0x453DE4-0x453DF8], which the
  system manager's init calls once [OFW-4.93 lv1 a:0x44B164].
- The doubleword at reply bytes 8 to 15 is +96 of the GameOS partition's
  object. The constructor sets it to 7, or to 0x0000100000000007 when
  the byte at vaddr 0xC000D256 is not 0
  [OFW-4.93 lv1 a:0x4501BC-0x4501FC]. The captured 7 agrees with the
  first case.
- Reply bytes 1 and 2 are bytes 2 and 3 at vaddr 0xC000D35B. The
  function at [OFW-4.93 lv1 a:0x45241C-0x452428] sets them. One caller
  gives it the byte at +24 of the partition object and a byte of the
  system manager's state [OFW-4.93 lv1 a:0x44F3D0-0x44F3DC]. The
  constructor of the captured object sets +24 to 1
  [OFW-4.93 lv1 a:0x4501D4], and the captured byte 1 is 1.

I did not find which code calls the kernel's receive loop, or each
place that can send a second request of payload byte 1 = 2 after boot.
A later request would get the same word at bytes 4 to 7. The other
bytes come from partition state that the reading above gives for the
GameOS partition.

## Files

| File | Rows | Holds |
|---|---|---|
| `reply.tsv` | 5 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |
| `stored.tsv` | 7 | the same columns, for the fields of the 0x0A reply |

The fields are `toc` (the TOC word), `manager` (the TOC slot r2 + 3864
of the object), and `reply2032`, `reply2040` and `reply2048` (the
doublewords at those offsets of the object). In `stored.tsv`,
`stored488` to `stored512` are the first four doublewords at +488 of
the object, and `count2024` is the doubleword at +2024.

## The live reply

- The TOC word and the TOC slot hold their stock values, so the object
  is at the address that the image gives.
- `reply2032` is 0x0100000000000000: reply byte 1 is 0x01, and reply
  byte 2 is 0x00.
- `reply2040` is 7: reply bytes 8 to 15.
- The high word of `reply2048` is 4: reply bytes 4 to 7. Its low word,
  +2052, is a field of the kernel that this reply does not set.

So the reply's 16 payload bytes are
`02 01 00 00 00 00 00 04 00 00 00 00 00 00 00 07`.

## The stored bytes

On 2026-11-14, the same command read the words of `stored.tsv` with the
same method: two reads of each page, which agree, and no write, poke or
hypercall. The same read gave the three words of `reply.tsv` again, and
they agree with that file.

### Where the kernel keeps them

1. Entry 7 of the receive loop's table, for service id 0x0A, takes the
   case at 0x8000000000264d9c
   [OFW-4.93 lv2_kernel a:0x8000000000264d9c-0x8000000000264da4].
2. That case copies min(the halfword at payload byte 2, 1536) bytes from
   payload byte 8 to +488 of the object. It stores that count as a
   doubleword at +2024, and posts event bit 2
   [OFW-4.93 lv2_kernel a:0x8000000000261ac0-0x8000000000261b20].
3. No other kernel instruction stores to +2024. The getter at
   [OFW-4.93 lv2_kernel a:0x8000000000261d50] waits for event bit 2,
   and then copies the bytes out
   [OFW-4.93 lv2_kernel a:0x8000000000261da8-0x8000000000261dd4].

### Where lv1 gets them

lv1's handler of service id 9 replies to payload byte 1 = 1 with service
id 0x0A [OFW-4.93 lv1 a:0x44DF2C-0x44DF60]:
- payload byte 0 is 1, byte 1 is 0, and bytes 4 to 7 are 0;
- bytes 2 and 3 are the low halfword of the count at the system
  manager's vaddr 0xC000D1A0;
- the header counts 8 + the count.

It then writes the count's bytes from the 1536-byte buffer whose address
is at vaddr 0xC000D1A8 [OFW-4.93 lv1 a:0x44CFC0-0x44CFF8], and takes each
write from the count [OFW-4.93 lv1 a:0x44DF64-0x44DFC8].
So the count is 0 after one reply.

The count has these writers:
- The system manager's init stores 128, and calls
  [OFW-4.93 lv1 a:0x4527B0] [OFW-4.93 lv1 a:0x44CFC0-0x44D000]. That
  function sets the count to byte 5 of the block at vaddr 0xC000D35B
  when byte 5 is not above 128, and to 0 otherwise
  [OFW-4.93 lv1 a:0x4527B0-0x452818]. So init gives at most 128.
- The handler of a request of service id 0x0A sets the count to the
  request's halfword at payload byte 2, at most 1536
  [OFW-4.93 lv1 a:0x44D448-0x44D470]. Entry 10 of the handler table
  holds its descriptor [OFW-4.93 lv1 a:0x46FDB0-0x46FDB4]
  [OFW-4.93 lv1 a:0x4776A0-0x4776A4].

The kernel sends that request in one place: `sys_sm_shutdown`, with the
caller's parameter of at most 1536 bytes
[OFW-4.93 lv2_kernel a:0x800000000000aa04-0x800000000000aae8]
[OFW-4.93 lv2_kernel a:0x8000000000264798-0x8000000000264878]. So the
bytes pass from one GameOS boot to the next while lv1 runs.

### The live bytes

- `count2024` is 0x600: the kernel kept 1536 bytes.
- The first 32 of them are 0.

Init gives at most 128. So these bytes came from a request of service id
0x0A: a `sys_sm_shutdown` call of an earlier GameOS boot with a
parameter of 1536 bytes or more.

At the first GameOS boot after lv1 starts, no such request came first.
At that boot, the count is byte 5 of the block. The system controller
setup clears the block's 256 bytes. It fills them from the system
controller only when bit 0 of the word at vaddr 0xC000D2D8 is set
[OFW-4.93 lv1 a:0x453E40-0x453E64]. That word is 4 (see `reply2048`
above). The init runs the setup before it stores the count
[OFW-4.93 lv1 a:0x44B164-0x44B170]. The other writers of byte 5 are
these:
- a function that clears it [OFW-4.93 lv1 a:0x4523DC-0x4523E4];
- [OFW-4.93 lv1 a:0x45281C-0x452850], which only the handler of service
  id 0x0A calls [OFW-4.93 lv1 a:0x44D490].

So byte 5 is 0, and the first boot's reply counts 0 bytes.
