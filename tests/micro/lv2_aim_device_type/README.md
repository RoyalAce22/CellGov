# lv2_aim_device_type

A read-only capture from the 4.93 kernel's memory on the reference
console. It gives the reply that the kernel kept from its first request
to lv1's dispatcher manager: request 0x19002 of service 0x19000, which
the kernel sends on virtual UART port 10 at boot.

lv1 does not hold the reply's 16 data bytes. The service behind 0x19000
builds them with an isolated SPU module over data of the console
(`cellgov_ps3_abi::lv1::uart`, "The reply to the kernel's first
request"). The raw exchange does not stay in memory after boot, but the
kernel keeps the reply in a cache object. This capture reads that object.
The test `uart_reply_console_tests` holds
`cellgov_ps3_abi::lv1::uart::GAME_OS_DISPATCHER_REPLIES` to this capture.

No probe runs, so this directory has no manifest and no runner frame.
The file under `peek/cech20-cex-493/` holds the words the reads returned.

## Method

- The console is the reference profile `cech20-cex-493`: a CECH-2001A on
  retail 4.93 with custom firmware (EvilNAT 4.93 PEX, Cobra 8.5) and
  webMAN MOD 1.47.48.
- On 2026-11-13, `cellgov ps3 peek --space lv2 --width 8` read the
  doublewords below through webMAN's `peek.lv2` command, after the
  console had booted to the XMB. It read each page twice, and the two
  reads agree. It wrote nothing to lv1 or lv2, used no poke, sent no
  hypercall and installed nothing on the console.
- The kernel stores to each word of the cache object that this capture
  reads, so its page is writable
  [OFW-4.93 lv2_kernel a:0x800000000022aaf0-0x800000000022ab14]. The
  capture `lv2_vuart_port_params` read the same page.
- No row of `tests/micro/console_crashes.tsv` covers these words.
- No console-identifying value is in this directory.

## Where the kernel keeps the reply

The shipped 4.93 `lv2_kernel.self`, decrypted offline and read with
`cellgov dev disasm`, gives these steps:

- The request object's constructor puts the request id 0x19002 at +8
  and the service id 0x19000 at +16, and zeroes +24 to +65
  [OFW-4.93 lv2_kernel a:0x8000000000229938-0x8000000000229994].
- The base unmarshal checks that the reply starts with the request id,
  and copies the reply's word at +16, the status, to +24 of the object
  [OFW-4.93 lv2_kernel a:0x8000000000212d18-0x8000000000212d6c]. The
  class unmarshal copies the reply's 16 bytes at +40 to +50 of the
  object [OFW-4.93 lv2_kernel a:0x8000000000229ab0-0x8000000000229b38].
- The getter takes the cache object from the TOC slot r2 + 1040
  [OFW-4.93 lv2_kernel a:0x800000000022a778]. When the byte at +0 of
  the cache is 0, it sends the request, and copies the 16 bytes to +4
  to +19 of the cache. It sets the byte at +0 to 1 only when the status
  is 0 [OFW-4.93 lv2_kernel a:0x800000000022aac0-0x800000000022ab14].
- When the byte at +0 is 1, the getter gives the kept bytes and sends no
  request [OFW-4.93 lv2_kernel a:0x800000000022a9ac-0x800000000022aa58].

## Files

| File | Rows | Holds |
|---|---|---|
| `aim_cache.tsv` | 5 | for each word: its address, its field, its width in bytes, the number of reads, whether they agree, and its value |

The fields are:

- `toc`: the TOC word.
- `cache`: the TOC slot r2 + 1040, which holds the cache object's
  address.
- `cache0`, `cache8` and `cache16`: the doublewords at +0, +8 and +16 of
  the cache object.

## The kept reply

- The TOC word is the value that the earlier virtual UART captures read.
  The TOC slot holds the cache object's address.
- The byte at +0 of the cache is 1. So the status of the console's reply
  was 0.
- The bytes at +4 to +19 of the cache are the reply's 16 bytes at +40:
  15 bytes of 0, then 0x84.
- The bytes at +1 to +3 and +20 to +23 belong to the kernel's other
  requests of the same service. This capture does not use them.

`sysmgr_ss.fself` of lv1's process archive sends the same request, and
compares the last 8 of the 16 bytes, as one doubleword, with 0x82
[OFW-4.93 lv1 a:0x463B18-0x463B40]. So the bytes hold a value that
names a class of console. CellGov answers the request with the value of
the reference console's class.
