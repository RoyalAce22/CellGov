/* SPU program for spu_issue_runout, run once on a raw SPU.
 *
 * The PPU writes the effective address of a 16 KB buffer to the word at
 * 0x7F00 before the run.
 *
 * Each case times one loop with the decrementer and stores the count it
 * fell by, in the preferred word of slot k, 16 bytes each from 0x8000.
 * Case k's code starts at 0x400 + k * 0x800; its loop starts at 0x180
 * past that, 128-byte aligned, and its loop branch is hinted. Cases 0 to
 * 5 run 65536 iterations; cases 6 to 9 run 512.
 *
 * Loads and stores take the local store in every cycle they issue, so a
 * long run of them leaves inline prefetch no free slot:
 *
 *   0  64 lnop
 *   1  64 lqa
 *   2  64 stqa
 *   3  32 nop + lnop pairs
 *   4  32 nop + lqa pairs
 *   5  128 lqa
 *
 * Cases 6 and 8 first queue sixteen 16 KB gets from the buffer to the
 * local store at 0x10000 under tag 1, then time their loop while the
 * gets run; cases 7 and 9 time the same loop with nothing queued:
 *
 *   6  64 lnop, gets queued    7  64 lnop
 *   8  64 lqa, gets queued     9  64 lqa
 *
 * Slots 10 and 11 hold the count from the start of case 6's and case
 * 8's loop to the end of their wait for tag 1, which shows whether the
 * gets outlasted the loop.
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	ilhu	$4, 0x7FFF
	iohl	$4, 0xFFFF
	wrch	$ch7, $4
	lqa	$31, 0x7F00
	il	$32, 0
	wrch	$ch17, $32
	ilhu	$33, 1
	il	$34, 0x4000
	il	$35, 1
	il	$36, 0x40
	il	$37, 2
	il	$38, 2
	bra	0x400

	/* Start case k's code. */
	.macro	CASE k
	.org	0x400 + (\k) * 0x800
	.endm

	/* Read the decrementer and enter case k's hinted loop. */
	.macro	LOOP k
	hbrr	B\k, L\k
	rdch	$20, $ch8
	br	L\k
	.org	0x400 + (\k) * 0x800 + 0x180
L\k:
	.endm

	/* Close case k's loop and store the count it fell by. */
	.macro	CLOSE k
	ai	$3, $3, -1
B\k:
	brnz	$3, L\k
	rdch	$21, $ch8
	sf	$22, $21, $20
	stqa	$22, 0x8000 + (\k) * 16
	.endm

	.macro	NEXT k
	bra	0x400 + ((\k) + 1) * 0x800
	.endm

	/* Queue sixteen 16 KB gets under tag 1. */
	.macro	GETS
	.rept	16
	wrch	$ch16, $33
	wrch	$ch18, $31
	wrch	$ch19, $34
	wrch	$ch20, $35
	wrch	$ch21, $36
	.endr
	.endm

	/* Wait for tag 1 and store the count since the loop started. */
	.macro	WAIT slot
	wrch	$ch22, $37
	wrch	$ch23, $38
	rdch	$39, $ch24
	rdch	$21, $ch8
	sf	$22, $21, $20
	stqa	$22, 0x8000 + (\slot) * 16
	.endm

	CASE	0
	ilhu	$3, 1
	LOOP	0
	.rept	64
	lnop
	.endr
	CLOSE	0
	NEXT	0

	CASE	1
	ilhu	$3, 1
	LOOP	1
	.rept	64
	lqa	$10, 0x9000
	.endr
	CLOSE	1
	NEXT	1

	CASE	2
	ilhu	$3, 1
	LOOP	2
	.rept	64
	stqa	$11, 0x9000
	.endr
	CLOSE	2
	NEXT	2

	CASE	3
	ilhu	$3, 1
	LOOP	3
	.rept	32
	nop	$0
	lnop
	.endr
	CLOSE	3
	NEXT	3

	CASE	4
	ilhu	$3, 1
	LOOP	4
	.rept	32
	nop	$0
	lqa	$10, 0x9000
	.endr
	CLOSE	4
	NEXT	4

	CASE	5
	ilhu	$3, 1
	LOOP	5
	.rept	128
	lqa	$10, 0x9000
	.endr
	CLOSE	5
	NEXT	5

	CASE	6
	GETS
	il	$3, 512
	LOOP	6
	.rept	64
	lnop
	.endr
	CLOSE	6
	WAIT	10
	NEXT	6

	CASE	7
	il	$3, 512
	LOOP	7
	.rept	64
	lnop
	.endr
	CLOSE	7
	NEXT	7

	CASE	8
	GETS
	il	$3, 512
	LOOP	8
	.rept	64
	lqa	$10, 0x9000
	.endr
	CLOSE	8
	WAIT	11
	NEXT	8

	CASE	9
	il	$3, 512
	LOOP	9
	.rept	64
	lqa	$10, 0x9000
	.endr
	CLOSE	9
	NEXT	9

	CASE	10
	stop	0x2000

	.org	0x8000
slots:
	.rept	12 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
