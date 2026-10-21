/* SPU program for spu_issue_fetch, run once on a raw SPU.
 *
 * Each case times one loop of 65536 iterations with the decrementer and
 * stores the count it fell by, in the preferred word of slot k, 16 bytes
 * each from 0x8000. Case k's code starts at 0x400 + k * 0x600; its loop
 * starts at 0x80 past that, 128-byte aligned, except case 15's, and its
 * loop branch is hinted.
 *
 * A load or store takes the local store in the cycle it issues, and
 * inline prefetch waits for a cycle no load or store takes. Each loop is
 * a run of lqa or stqa, then ai and brnz:
 *
 *   0  64 lqa      1  72 lqa      2  80 lqa      3  88 lqa
 *   4  96 lqa      5  104 lqa     6  112 lqa     7  120 lqa
 *   8  128 lqa     9  160 lqa     10 192 lqa     11 224 lqa
 *   12 128 stqa
 *   13 64 lqa, lnop, 64 lqa
 *   14 four times 32 lqa and lnop
 *   15 128 lqa, the loop 64 bytes past a 128-byte line
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	ilhu	$4, 0x7FFF
	iohl	$4, 0xFFFF
	wrch	$ch7, $4
	bra	0x400

	/* Start case k's code and enter its hinted loop at `at` past it. */
	.macro	LOOP k, at=0x80
	.org	0x400 + (\k) * 0x600
	ilhu	$3, 1
	rdch	$20, $ch8
	hbrr	B\k, L\k
	br	L\k
	.org	0x400 + (\k) * 0x600 + (\at)
L\k:
	.endm

	/* Close case k's loop, store the count it fell by and go on. */
	.macro	CLOSE k
	ai	$3, $3, -1
B\k:
	brnz	$3, L\k
	rdch	$21, $ch8
	sf	$22, $21, $20
	stqa	$22, 0x8000 + (\k) * 16
	bra	0x400 + ((\k) + 1) * 0x600
	.endm

	.macro	LOADS n
	.rept	\n
	lqa	$10, 0x9000
	.endr
	.endm

	LOOP	0
	LOADS	64
	CLOSE	0

	LOOP	1
	LOADS	72
	CLOSE	1

	LOOP	2
	LOADS	80
	CLOSE	2

	LOOP	3
	LOADS	88
	CLOSE	3

	LOOP	4
	LOADS	96
	CLOSE	4

	LOOP	5
	LOADS	104
	CLOSE	5

	LOOP	6
	LOADS	112
	CLOSE	6

	LOOP	7
	LOADS	120
	CLOSE	7

	LOOP	8
	LOADS	128
	CLOSE	8

	LOOP	9
	LOADS	160
	CLOSE	9

	LOOP	10
	LOADS	192
	CLOSE	10

	LOOP	11
	LOADS	224
	CLOSE	11

	LOOP	12
	.rept	128
	stqa	$11, 0x9000
	.endr
	CLOSE	12

	LOOP	13
	LOADS	64
	lnop
	LOADS	64
	CLOSE	13

	LOOP	14
	.rept	4
	LOADS	32
	lnop
	.endr
	CLOSE	14

	LOOP	15, 0xC0
	LOADS	128
	CLOSE	15

	.org	0x400 + 16 * 0x600
	stop	0x2000

	.org	0x8000
slots:
	.rept	16 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
