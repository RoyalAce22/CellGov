/* SPU program for spu_tables, run on one raw SPU from four entry points.
 *
 * 0x000  The channel sweep: for channel n = 0..127, the block at 8 * n
 *        does rchcnt on channel n and stores the count to 0x2000 + 16 n.
 *        The PPU restarts at the next block when one stops the SPU. The
 *        sweep ends with stop 0x2001.
 * 0x800  The in mailbox: rchcnt 29 to 0x2800, four rdch 29 to 0x2810..
 *        0x2840, rchcnt 29 to 0x2850; stop 0x2002.
 * 0x900  The out mailbox: rchcnt 28 to 0x2880, one wrch 28 of 0x0BB01571,
 *        rchcnt 28 to 0x2890; stop 0x2003.
 * 0xA00  The MFC commands. The PPU writes the effective address of a
 *        4 KB, 128-byte-aligned buffer to 0x1F00. Command k uses the
 *        128-byte line at buffer + 0x80 k and local store 0x4000 + 0x80 k,
 *        16 bytes, tag 1, after the SPU stores 0x5000000k to the local
 *        line. After each tagged command the tag status (wait for all
 *        tags) goes to 0x3000 + 16 k; after each atomic command the
 *        MFC_RdAtomicStat word goes to 0x3100 + 16 k. Commands:
 *          0 put 0x20    1 putb 0x21   2 putf 0x22   3 putr 0x30
 *          4 putrb 0x31  5 putrf 0x32  6 get 0x40    7 getb 0x41
 *          8 getf 0x42   9 barrier 0xC0  10 mfceieio 0xC8  11 mfcsync 0xCC
 *         12 getllar 0xD0 on line 12
 *         13 putllc 0xB4 on line 12, after the SPU stores 0x5000000D to
 *            local line 12
 *         14 putlluc 0xB0 on line 14  15 putqlluc 0xB8 on line 15
 *        stop 0x2004.
 */
	.text
	.global	_start
_start:
	.irp	n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127
	rchcnt	$3, $ch\n
	stqa	$3, 0x2000 + (\n) * 16
	.endr
	stop	0x2001

	.org	0x800
mbox_in:
	rchcnt	$3, $ch29
	stqa	$3, 0x2800
	rdch	$4, $ch29
	stqa	$4, 0x2810
	rdch	$4, $ch29
	stqa	$4, 0x2820
	rdch	$4, $ch29
	stqa	$4, 0x2830
	rdch	$4, $ch29
	stqa	$4, 0x2840
	rchcnt	$3, $ch29
	stqa	$3, 0x2850
	stop	0x2002

	.org	0x900
mbox_out:
	rchcnt	$3, $ch28
	stqa	$3, 0x2880
	ilhu	$4, 0x0BB0
	iohl	$4, 0x1571
	wrch	$ch28, $4
	rchcnt	$3, $ch28
	stqa	$3, 0x2890
	stop	0x2003

	.org	0xA00
mfc:
	lqa	$11, 0x1F00
	il	$15, -1
	il	$16, 2
	il	$9, 0
	wrch	$ch17, $9

	/* Local line k starts with 0x5000000k. */
	.irp	k, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
	ilhu	$3, 0x5000
	iohl	$3, (\k)
	stqa	$3, 0x4000 + (\k) * 0x80
	.endr

	/* Point the parameter channels at line k. */
	.macro	PARAMS k
	il	$4, 0x4000 + (\k) * 0x80
	wrch	$ch16, $4
	il	$6, (\k) * 0x80
	a	$5, $11, $6
	wrch	$ch18, $5
	il	$7, 16
	wrch	$ch19, $7
	il	$8, 1
	wrch	$ch20, $8
	.endm

	/* Wait for every tag group; the tag status goes to slot k. */
	.macro	WAIT_ALL k
	wrch	$ch22, $15
	wrch	$ch23, $16
	rdch	$25, $ch24
	stqa	$25, 0x3000 + (\k) * 16
	.endm

	/* A tagged command k with opcode op. */
	.macro	TAGGED k, op
	PARAMS	\k
	il	$10, \op
	wrch	$ch21, $10
	WAIT_ALL \k
	.endm

	/* An atomic command k with opcode op on line ln. */
	.macro	ATOMIC k, op, ln
	PARAMS	\ln
	il	$10, \op
	wrch	$ch21, $10
	rdch	$12, $ch27
	stqa	$12, 0x3100 + (\k) * 16
	.endm

	TAGGED	0, 0x20
	TAGGED	1, 0x21
	TAGGED	2, 0x22
	TAGGED	3, 0x30
	TAGGED	4, 0x31
	TAGGED	5, 0x32
	TAGGED	6, 0x40
	TAGGED	7, 0x41
	TAGGED	8, 0x42
	TAGGED	9, 0xC0
	TAGGED	10, 0xC8
	TAGGED	11, 0xCC
	ATOMIC	12, 0xD0, 12
	ilhu	$3, 0x5000
	iohl	$3, 13
	stqa	$3, 0x4000 + 12 * 0x80
	ATOMIC	13, 0xB4, 12
	ATOMIC	14, 0xB0, 14
	TAGGED	15, 0xB8
	stop	0x2004

	/* Every result slot starts as 0xEEEEEEEE words. */
	.org	0x2000
	.rept	(0x2900 - 0x2000) / 4
	.long	0xEEEEEEEE
	.endr
	.org	0x3000
	.rept	0x200 / 4
	.long	0xEEEEEEEE
	.endr
