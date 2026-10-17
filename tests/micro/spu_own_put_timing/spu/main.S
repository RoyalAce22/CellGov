/* SPU program for spu_own_put_timing, run once on a raw SPU.
 *
 * The PPU writes the effective address of four 256-byte blocks to the
 * word at 0x1F00 before the run. Case k reserves the first line of
 * block k with getllar, issues one command of its own with tag 1, and
 * then tries a putllc of the line:
 *
 *   0  sndsig of 4 bytes from 0x3000 to the line
 *   1  put of 16 bytes from 0x3000 to the line
 *   2  put of 16 bytes from 0x3000 to the line, then a second getllar
 *      of the line before the wait for tag 1
 *   3  none
 *
 * SPU_WrEventMask is LR (0x400). At each drain the program reads the
 * SPU_RdEventStat count with rchcnt and, when it is not 0, reads
 * SPU_RdEventStat and acks what it read.
 *
 * Case k stores to slots 16k + n, 16 bytes each from 0x1000:
 *   0      the getllar status
 *   1, 2   the count and the event bits after the getllar
 *   3, 4   the count and the event bits right after the command issues
 *   5      the second getllar status (case 2 only)
 *   6, 7   the count and the event bits after the wait for tag 1
 *   8      the putllc status
 *   9, 10  the count and the event bits after the putllc
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	lqa	$11, 0x1F00
	il	$9, 0
	il	$10, 0x2000
	il	$15, 1
	il	$16, 2
	il	$17, 2
	il	$18, 0x400
	il	$40, 0xD0
	il	$41, 0xB4
	wrch	$ch17, $9
	wrch	$ch2, $18
	wrch	$ch1, $18

	.macro	DRAIN at
	rchcnt	$31, $ch0
	il	$33, 0
	brz	$31, d\@
	rdch	$33, $ch0
	wrch	$ch2, $33
d\@:
	stqa	$31, \at
	stqa	$33, \at + 0x10
	.endm

	.macro	GETLLAR at
	wrch	$ch16, $10
	wrch	$ch18, $12
	wrch	$ch21, $40
	rdch	$20, $ch27
	stqa	$20, \at
	.endm

	.macro	CASE k, op, size, reget
	ila	$19, \k*256
	a	$12, $11, $19
	GETLLAR	slots+(\k)*0x100
	DRAIN	slots+(\k)*0x100+0x10
	.if	\op
	il	$23, 0x3000
	wrch	$ch16, $23
	wrch	$ch18, $12
	il	$22, \size
	wrch	$ch19, $22
	wrch	$ch20, $15
	il	$24, \op
	wrch	$ch21, $24
	.endif
	DRAIN	slots+(\k)*0x100+0x30
	.if	\reget
	GETLLAR	slots+(\k)*0x100+0x50
	.endif
	wrch	$ch22, $16
	wrch	$ch23, $17
	rdch	$25, $ch24
	DRAIN	slots+(\k)*0x100+0x60
	wrch	$ch16, $10
	wrch	$ch18, $12
	wrch	$ch21, $41
	rdch	$27, $ch27
	stqa	$27, slots + (\k)*0x100 + 0x80
	DRAIN	slots+(\k)*0x100+0x90
	.endm

	CASE	0, 0xA0, 4, 0
	CASE	1, 0x20, 16, 0
	CASE	2, 0x20, 16, 1
	CASE	3, 0, 0, 0

	stop	0x2000

	.org	0x1000
slots:
	.rept	64 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
