/* SPU program for spu_run_control, run on a raw SPU.
 *
 * Twenty-one entries, 0x800 bytes apart from _start. Entry k has sixteen
 * result slots of 16 bytes at 0x10000 + 0x100k, which start as
 * 0xEEEEEEEE words, and every entry that runs to its end stops with
 * 0x2000 + k.
 *
 * Entries 0 and 1, a met halt: `heq $9, $9` with $9 = 0, in the even
 * slot (entry 0) and the odd slot (entry 1). After it a loop counts $21
 * from 1 to 500 and stores the count to slot 0 on every pass, then
 * stops. Slot 0 is how far the SPU ran past the halt.
 *
 * Entries 2-9, instruction prefetch: a `stqr` at +0x0C writes the
 * quadword at 0xF000 (four `stop 0x3100` words) over the quadword X at
 * +0x10 + 16d, with d = 0, 1, 2, 4, 8, 16, 32, 64 quadwords of nops
 * between. X holds four `ai $20, $20, 1`; the code after X stores $20
 * to slot 0 and the word at X, read as data, to slot 1. The SPU stops
 * with 0x3100 when it fetched the new X, and stores 4 when it ran the
 * old one.
 *
 * Entries 10 and 11: entry 2's shape with the store at +0x08 and `sync`
 * (entry 10) or `dsync` (entry 11) at +0x0C, before X.
 *
 * Entry 12, the interrupt-enable bits: each branch goes to the next
 * word and is followed by an rdch of SPU_RdMachStat (13) into the next
 * slot. The branches are `bie`, then one with D and E both set, then
 * `bid`, then D and E again, then `bie` and `bid`, in slots 0-5.
 *
 * Entries 14-20: entry 2's shape with d = 9 to 15.
 *
 * Entry 13, the SPRs: $3 = 0x55, then `mfspr $3, sprN` into slot N for
 * N = 0-7, then `mtspr spr0, $9` with $9 = 0x1234 and `mfspr` of spr0
 * into slot 8.
 */
	.text
	.global	_start
_start:

	.macro	HALT k, odd
	.org	\k * 0x800
	il	$21, 0
	il	$9, 0
	.if	\odd
	nop
	.endif
	heq	$9, $9
loop\@:
	ai	$21, $21, 1
	stqr	$21, slots + \k * 0x100
	ceqi	$22, $21, 500
	brz	$22, loop\@
	stop	0x2000 + \k
	.endm

	.macro	PREFETCH k, d
	.org	\k * 0x800
	il	$20, 0
	lqr	$10, patch
	nop
	stqr	$10, x\@
	.rept	\d
	nop
	lnop
	nop
	lnop
	.endr
x\@:
	ai	$20, $20, 1
	ai	$20, $20, 1
	ai	$20, $20, 1
	ai	$20, $20, 1
	stqr	$20, slots + \k * 0x100
	lqr	$11, x\@
	stqr	$11, slots + \k * 0x100 + 0x10
	stop	0x2000 + \k
	.endm

	.macro	SYNCED k, insn
	.org	\k * 0x800
	il	$20, 0
	lqr	$10, patch
	stqr	$10, x\@
	\insn
x\@:
	ai	$20, $20, 1
	ai	$20, $20, 1
	ai	$20, $20, 1
	ai	$20, $20, 1
	stqr	$20, slots + \k * 0x100
	lqr	$11, x\@
	stqr	$11, slots + \k * 0x100 + 0x10
	stop	0x2000 + \k
	.endm

	/* Branch indirect $5 to the next word with E (0x40000), D
	 * (0x80000) or both, then store SPU_RdMachStat to slot n. */
	.macro	IEBRANCH k, bits, n
	ila	$5, ie\@
	.long	0x35000000 | \bits | (5 << 7)
ie\@:
	rdch	$3, $ch13
	stqr	$3, slots + \k * 0x100 + \n * 0x10
	.endm

	HALT	0, 0
	HALT	1, 1
	PREFETCH 2, 0
	PREFETCH 3, 1
	PREFETCH 4, 2
	PREFETCH 5, 4
	PREFETCH 6, 8
	PREFETCH 7, 16
	PREFETCH 8, 32
	PREFETCH 9, 64
	SYNCED	10, sync
	SYNCED	11, dsync

	.org	12 * 0x800
	IEBRANCH 12, 0x40000, 0
	IEBRANCH 12, 0xC0000, 1
	IEBRANCH 12, 0x80000, 2
	IEBRANCH 12, 0xC0000, 3
	IEBRANCH 12, 0x40000, 4
	IEBRANCH 12, 0x80000, 5
	stop	0x2000 + 12

	/* mfspr $3, sprN is 0x01800000 | N << 7 | 3; mtspr sprN, $9 is
	 * 0x21800000 | N << 7 | 9. */
	.org	13 * 0x800
	il	$9, 0x1234
	.irp	n, 0, 1, 2, 3, 4, 5, 6, 7
	il	$3, 0x55
	.long	0x01800000 | (\n << 7) | 3
	stqr	$3, slots + 13 * 0x100 + \n * 0x10
	.endr
	.long	0x21800000 | (0 << 7) | 9
	il	$3, 0x55
	.long	0x01800000 | (0 << 7) | 3
	stqr	$3, slots + 13 * 0x100 + 8 * 0x10
	stop	0x2000 + 13

	PREFETCH 14, 9
	PREFETCH 15, 10
	PREFETCH 16, 11
	PREFETCH 17, 12
	PREFETCH 18, 13
	PREFETCH 19, 14
	PREFETCH 20, 15

	.org	0xF000
patch:
	.long	0x3100, 0x3100, 0x3100, 0x3100

	.org	0x10000
slots:
	.rept	21 * 0x100 / 4
	.long	0xEEEEEEEE
	.endr
