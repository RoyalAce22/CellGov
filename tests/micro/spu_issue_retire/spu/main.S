/* SPU program for spu_issue_retire, run once on a raw SPU.
 *
 * Each case times one loop of 65536 iterations with the decrementer
 * and stores the count it fell by, in the preferred word of slot k, 16
 * bytes each from 0x8000. Case k's code starts at 0x400 + k * 0x200;
 * its loop starts at 0x500 past that, 128-byte aligned, and its loop
 * branch is hinted.
 *
 * Every instruction in a loop is on the even pipeline except the loop's
 * brnz, and only the ai and the brnz depend on each other. A case puts
 * a 2-cycle ai or a either 4 cycles after a 6-cycle fa or fm, which
 * would complete with it, or 3 or 5 cycles after it, which would not.
 *
 *   0  fa / nop / nop / nop / ai / brnz: the ai completes with the fa
 *   1  fa / nop / nop / ai / nop / brnz: it does not
 *   2  fm / a / a / a / ai / brnz: the ai completes with the fm
 *   3  fm / a / a / ai / a / brnz: it does not
 *   4  fa / nop / nop / nop / nop / ai / brnz: the ai 5 cycles after
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	ilhu	$4, 0x7FFF
	iohl	$4, 0xFFFF
	wrch	$ch7, $4
	il	$5, 0
	il	$6, 0
	il	$7, 0
	bra	0x400

	.macro	HEAD k
	.org	0x400 + (\k) * 0x200
	ilhu	$3, 1
	hbrr	B\k, L\k
	rdch	$20, $ch8
	br	L\k
	.org	0x400 + (\k) * 0x200 + 0x100
L\k:
	.endm

	.macro	TAIL k
B\k:
	brnz	$3, L\k
	rdch	$21, $ch8
	sf	$22, $21, $20
	stqa	$22, 0x8000 + (\k) * 16
	bra	0x400 + ((\k) + 1) * 0x200
	.endm

	HEAD	0
	fa	$10, $5, $6
	nop	$0
	nop	$0
	nop	$0
	ai	$3, $3, -1
	TAIL	0

	HEAD	1
	fa	$10, $5, $6
	nop	$0
	nop	$0
	ai	$3, $3, -1
	nop	$0
	TAIL	1

	HEAD	2
	fm	$10, $5, $6
	a	$11, $6, $7
	a	$12, $6, $7
	a	$13, $6, $7
	ai	$3, $3, -1
	TAIL	2

	HEAD	3
	fm	$10, $5, $6
	a	$11, $6, $7
	a	$12, $6, $7
	ai	$3, $3, -1
	a	$13, $6, $7
	TAIL	3

	HEAD	4
	fa	$10, $5, $6
	nop	$0
	nop	$0
	nop	$0
	nop	$0
	ai	$3, $3, -1
	TAIL	4

	.org	0x400 + 5 * 0x200
	stop	0x2000

	.org	0x8000
slots:
	.rept	8 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
