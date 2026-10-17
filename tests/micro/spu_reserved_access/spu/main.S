/* SPU program for spu_reserved_access, run on one raw SPU from 17 entry
 * points.
 *
 * Reserved channels, in case order: 5, 6, 10, 31, 69, 70, 71, 127.
 *
 * Case k (0..7) at 0x20 k reads reserved channel k:
 *   il $3, 0x55; rdch $3, $chN; stqa $3 to slot k; stop 0x2100 + k.
 * Case 8 + k at 0x20 (8 + k) writes it:
 *   il $3, 0x66; wrch $chN, $3; rchcnt $4, $chN; stqa $4 to slot 8 + k;
 *   stop 0x2108 + k.
 * Slot n is 16 bytes at 0x2000 + 16 n and starts as 0xEEEEEEEE words.
 *
 * 0x400  The storage control commands. The PPU writes the effective
 *        address of a 4 KB, 128-byte-aligned buffer to 0x1F00. Command k
 *        acts on the 128-byte line at buffer + 0x80 k, size 128, tag 1,
 *        local store 0x4000 + 0x80 k; a wait for every tag group follows
 *        and its status goes to 0x2200 + 16 k. Commands:
 *          0 sdcrt 0x80  1 sdcrtst 0x81  2 sdcrz 0x89  3 sdcrst 0x8D
 *          4 sdcrf 0x8F
 *        stop 0x2200.
 */
	.text
	.global	_start
_start:

	.macro	READ k, ch
	.org	(\k) * 0x20
	il	$3, 0x55
	rdch	$3, $ch\ch
	stqa	$3, 0x2000 + (\k) * 16
	stop	0x2100 + (\k)
	.endm

	.macro	WRITE k, ch
	.org	(8 + \k) * 0x20
	il	$3, 0x66
	wrch	$ch\ch, $3
	rchcnt	$4, $ch\ch
	stqa	$4, 0x2000 + (8 + \k) * 16
	stop	0x2108 + (\k)
	.endm

	READ	0, 5
	READ	1, 6
	READ	2, 10
	READ	3, 31
	READ	4, 69
	READ	5, 70
	READ	6, 71
	READ	7, 127
	WRITE	0, 5
	WRITE	1, 6
	WRITE	2, 10
	WRITE	3, 31
	WRITE	4, 69
	WRITE	5, 70
	WRITE	6, 71
	WRITE	7, 127

	.org	0x400
sl1:
	lqa	$11, 0x1F00
	il	$15, -1
	il	$16, 2
	il	$9, 0
	wrch	$ch17, $9

	.macro	CONTROL k, op
	il	$4, 0x4000 + (\k) * 0x80
	wrch	$ch16, $4
	il	$6, (\k) * 0x80
	a	$5, $11, $6
	wrch	$ch18, $5
	il	$7, 128
	wrch	$ch19, $7
	il	$8, 1
	wrch	$ch20, $8
	il	$10, \op
	wrch	$ch21, $10
	wrch	$ch22, $15
	wrch	$ch23, $16
	rdch	$25, $ch24
	stqa	$25, 0x2200 + (\k) * 16
	.endm

	CONTROL	0, 0x80
	CONTROL	1, 0x81
	CONTROL	2, 0x89
	CONTROL	3, 0x8D
	CONTROL	4, 0x8F
	stop	0x2200

	.org	0x2000
	.rept	0x300 / 4
	.long	0xEEEEEEEE
	.endr
