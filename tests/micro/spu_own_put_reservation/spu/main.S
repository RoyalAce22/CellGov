/* SPU program for spu_own_put_reservation, run once on a raw SPU.
 *
 * The PPU writes the effective address of nine 256-byte blocks to the
 * word at 0x1F00 before the run. Case k reserves the first line of
 * block k, issues one command of its own, and then tries a conditional
 * store to the line:
 *
 *   1. getllar of the line to 0x2000; read MFC_RdAtomicStat.
 *   2. the case's command, tag 1, and a wait for tag 1.
 *   3. putllc of the line from 0x2000; read MFC_RdAtomicStat.
 *
 * SPU_WrEventMask is LR (0x400). After the getllar, the command and
 * the putllc, the program reads the SPU_RdEventStat count with rchcnt
 * and, when it is not 0, reads SPU_RdEventStat and acks what it read.
 *
 * The commands, cases 0-8:
 *   0  none
 *   1  put of 16 bytes from 0x3000 to the line
 *   2  put of 16 bytes from 0x3000 to the next line of the block
 *   3  sdcrz of the line
 *   4  sdcrf of the line
 *   5  sdcrst of the line
 *   6  putl of one 16-byte element from 0x3000 to the line
 *   7  get of 16 bytes from the line to 0x3000
 *   8  none
 *
 * Case k stores eight words to slots 8k to 8k + 7, 16 bytes each from
 * 0x1000: the getllar status, the count and the event bits after the
 * getllar (0 when the count is 0), the count and the event bits after
 * the command, the putllc status, and the count and the event bits
 * after the putllc. The SPU stops with 0x2000 after the last case.
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

	.macro	RESERVE k
	ila	$19, \k*256
	a	$12, $11, $19
	wrch	$ch16, $10
	wrch	$ch18, $12
	wrch	$ch21, $40
	rdch	$20, $ch27
	stqa	$20, slots + (\k)*0x80
	DRAIN	slots+(\k)*0x80+0x10
	.endm

	.macro	COMMAND lsa, ea, size, op
	il	$23, \lsa
	wrch	$ch16, $23
	wrch	$ch18, \ea
	il	$22, \size
	wrch	$ch19, $22
	wrch	$ch20, $15
	il	$24, \op
	wrch	$ch21, $24
	wrch	$ch22, $16
	wrch	$ch23, $17
	rdch	$25, $ch24
	.endm

	.macro	CONDITIONAL k
	DRAIN	slots+(\k)*0x80+0x30
	wrch	$ch16, $10
	wrch	$ch18, $12
	wrch	$ch21, $41
	rdch	$27, $ch27
	stqa	$27, slots + (\k)*0x80 + 0x50
	DRAIN	slots+(\k)*0x80+0x60
	.endm

	RESERVE	0
	CONDITIONAL 0

	RESERVE	1
	COMMAND	0x3000, $12, 16, 0x20
	CONDITIONAL 1

	RESERVE	2
	ai	$21, $12, 128
	COMMAND	0x3000, $21, 16, 0x20
	CONDITIONAL 2

	RESERVE	3
	COMMAND	0x3000, $12, 128, 0x89
	CONDITIONAL 3

	RESERVE	4
	COMMAND	0x3000, $12, 128, 0x8F
	CONDITIONAL 4

	RESERVE	5
	COMMAND	0x3000, $12, 128, 0x8D
	CONDITIONAL 5

	/* The list element at 0x3800: size 16, then the line's address. */
	RESERVE	6
	il	$28, 16
	cwd	$29, 4($9)
	shufb	$30, $12, $28, $29
	stqa	$30, 0x3800
	il	$23, 0x3000
	wrch	$ch16, $23
	il	$21, 0x3800
	wrch	$ch18, $21
	il	$22, 8
	wrch	$ch19, $22
	wrch	$ch20, $15
	il	$24, 0x24
	wrch	$ch21, $24
	wrch	$ch22, $16
	wrch	$ch23, $17
	rdch	$25, $ch24
	CONDITIONAL 6

	RESERVE	7
	COMMAND	0x3000, $12, 16, 0x40
	CONDITIONAL 7

	RESERVE	8
	CONDITIONAL 8

	stop	0x2000

	.org	0x1000
slots:
	.rept	72 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
