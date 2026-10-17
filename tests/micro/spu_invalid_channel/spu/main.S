/* SPU program for spu_invalid_channel, run on a raw SPU.
 *
 * Four cases, 0x40 bytes apart from _start. Each one is a channel access
 * in the wrong direction, which the SPU ISA calls invalid: a read of a
 * write-only channel or a write to a read-only channel.
 *
 *   case 0  +0x08 rdch $3, SPU_WrOutMbox (0x1C)  +0x0C stqr $3 to mark 0
 *   case 1  +0x08 wrch SPU_RdInMbox (0x1D), $3   +0x0C stqr $3 to mark 1
 *   case 2  +0x08 stqr $8 to mark 2              +0x0C rdch $3, SPU_WrOutMbox
 *   case 3  +0x08 stqr $8 to mark 3              +0x0C wrch SPU_RdInMbox, $3
 *
 * Every case starts with `il $3, 0x55` and `il $8, 0x77` and stops with
 * signal 0x2000 + k at +0x10. Mark k is 16 bytes at 0x2000 + 16k and
 * starts as 0xEEEEEEEE words. In cases 0 and 1 the store after the access
 * records $3: what the read returned, or 0x55 when the write left it. In
 * cases 2 and 3 the store before the access fills the mark with 0x77.
 */
	.text
	.global	_start
_start:

	.macro	CASE k, insn, slot
	.org	\k * 0x40
	il	$3, 0x55
	il	$8, 0x77
	.if	\slot == 0
	\insn
	stqr	$3, marks+\k*16
	.else
	stqr	$8, marks+\k*16
	\insn
	.endif
	stop	0x2000 + \k
	.endm

	CASE	0, "rdch $3, $ch28", 0
	CASE	1, "wrch $ch29, $3", 0
	CASE	2, "rdch $3, $ch28", 1
	CASE	3, "wrch $ch29, $3", 1

	.org	0x2000
marks:
	.rept	16
	.long	0xEEEEEEEE
	.endr
