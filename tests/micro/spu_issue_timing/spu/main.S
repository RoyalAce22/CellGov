/* SPU program for spu_issue_timing, run once on a raw SPU.
 *
 * Each case times one loop of 65536 iterations with the decrementer
 * and stores the count it fell by, in the preferred word of slot k, 16
 * bytes each from 0x8000. Case k's code starts at 0x400 + k * 0x200;
 * its loop starts at 0x100 + off past that, so the loop's address
 * modulo 128 is off. Each case reads the decrementer, branches to its
 * loop, and reads the decrementer again after the loop exits.
 *
 * A hinted case issues hbrr for its loop branch before the first read.
 * The event mask stays 0, so an rchcnt of SPU_RdEventStat reads 0.
 *
 *   0      nop / ai / brnz, hinted, off 0x3C
 *   1..8   nop / ai / brnz, not hinted, off 0x00 0x04 0x08 0x0C 0x10
 *          0x20 0x30 0x3C
 *   9      the wait_event poll loop of spu_events: ceq / brnz (not
 *          taken) / ai / rchcnt / nop / brz, not hinted, off 0x28
 *          (0x68 modulo 128)
 *   10     rchcnt / a (on the count) / ai / brnz, hinted
 *   11     rchcnt / rchcnt / ai / brnz, hinted
 *   12     rchcnt / 6 nop / rchcnt / 6 nop / ai / brnz, hinted
 *   13     sync / ai / brnz, hinted
 *   14     dsync / ai / brnz, hinted
 *   15     syncc / ai / brnz, hinted
 *   16     dfa / ai / brnz, hinted
 *   17     hbrr for an inner brnz that is not taken, 8 nop, the inner
 *          brnz, ai, and a loop brnz that is not hinted
 *   18     case 17 with lnop in place of the hbrr
 *   19     hbrr for the loop brnz 3 instructions ahead, nop, ai, brnz
 *   20     hbrr for the loop brnz 12 instructions ahead, 10 nop, ai, brnz
 *   21     ai / lnop / nop / lnop / nop / lnop / nop / brnz, hinted, off 0
 *   22     case 21 at off 4
 *   23     lqa (a zero quadword) / a (on it) / ai / brnz, hinted
 *   24     rotqbyi / ai / brnz, hinted
 *   25     mpy (its own result) / ai / brnz, hinted
 *   26     brnz not taken, not hinted / ai / loop brnz, hinted
 *   27     nop / brnz not taken / ai / lnop / nop / lnop / nop / brnz,
 *          hinted, off 0: whether the pair after the branch dual issues
 *   28     rchcnt / brnz on the count, not taken / ai / brnz, hinted,
 *          off 4
 *   29     ceq / brnz on the result, not taken / ai / brnz, hinted, off 4
 *   30     case 9 with lqa (a zero quadword) in place of the rchcnt
 *   31     case 9 with the ai / rchcnt pair split: ai / lnop / nop /
 *          rchcnt
 *   32     case 9 with the rchcnt first: nop / rchcnt / ceq / brnz / ai /
 *          lnop / nop / brz
 *   33     case 32 with lqa in place of the rchcnt
 *   34     nop / brnz not taken / nop / lnop / ai / lnop / nop / loop
 *          brnz, not hinted, off 0
 *   35     nop / lnop / nop / brnz not taken / ai / lnop / nop / loop
 *          brnz, not hinted, off 0
 *   36..41 case 1 with its loop at 0x40 0x48 0x60 0x68 0x70 0x7C
 *          modulo 128
 *   42     case 9 with its loop at 0x28 modulo 128
 *
 * The SPU stops with 0x2000 after the last case.
 */
	.text
	.global	_start
_start:
	ilhu	$4, 0x7FFF
	iohl	$4, 0xFFFF
	wrch	$ch7, $4
	il	$4, 0
	il	$9, 1
	il	$5, 1
	il	$6, 0
	il	$7, 0
	bra	0x400

	.macro	HEAD k, off, hinted
	.org	0x400 + (\k) * 0x200
	ilhu	$3, 1
	.if	\hinted
	hbrr	B\k, L\k
	.endif
	rdch	$20, $ch8
	br	L\k
	.org	0x400 + (\k) * 0x200 + 0x100 + (\off)
L\k:
	.endm

	.macro	TAIL k
	rdch	$21, $ch8
	sf	$22, $21, $20
	stqa	$22, 0x8000 + (\k) * 16
	bra	0x400 + ((\k) + 1) * 0x200
	.endm

	.macro	LOOP3 k, off, hinted
	HEAD	\k, \off, \hinted
	nop	$0
	ai	$3, $3, -1
B\k:
	brnz	$3, L\k
	TAIL	\k
	.endm

	LOOP3	0, 0x3C, 1
	LOOP3	1, 0x00, 0
	LOOP3	2, 0x04, 0
	LOOP3	3, 0x08, 0
	LOOP3	4, 0x0C, 0
	LOOP3	5, 0x10, 0
	LOOP3	6, 0x20, 0
	LOOP3	7, 0x30, 0
	LOOP3	8, 0x3C, 0

	/* 9: the poll loop, entered at its compare. */
	HEAD	9, 0x28, 0
	il	$9, 0
	ilhu	$8, 1
	br	P9
	.org	0x400 + 9 * 0x200 + 0x168
P9:
	ceq	$22, $9, $8
	brnz	$22, X9
	ai	$9, $9, 1
	rchcnt	$10, $ch0
	nop	$127
	brz	$10, P9
X9:
	TAIL	9

	HEAD	10, 0, 1
	rchcnt	$10, $ch0
	a	$3, $3, $10
	ai	$3, $3, -1
B10:
	brnz	$3, L10
	TAIL	10

	HEAD	11, 0, 1
	rchcnt	$10, $ch0
	rchcnt	$11, $ch0
	ai	$3, $3, -1
B11:
	brnz	$3, L11
	TAIL	11

	HEAD	12, 0, 1
	rchcnt	$10, $ch0
	.rept	6
	nop	$0
	.endr
	rchcnt	$11, $ch0
	.rept	6
	nop	$0
	.endr
	ai	$3, $3, -1
B12:
	brnz	$3, L12
	TAIL	12

	HEAD	13, 0, 1
	sync
	ai	$3, $3, -1
B13:
	brnz	$3, L13
	TAIL	13

	HEAD	14, 0, 1
	dsync
	ai	$3, $3, -1
B14:
	brnz	$3, L14
	TAIL	14

	HEAD	15, 0, 1
	syncc
	ai	$3, $3, -1
B15:
	brnz	$3, L15
	TAIL	15

	HEAD	16, 0, 1
	dfa	$5, $6, $7
	ai	$3, $3, -1
B16:
	brnz	$3, L16
	TAIL	16

	HEAD	17, 0, 0
	hbrr	I17, X17
	.rept	8
	nop	$0
	.endr
I17:
	brnz	$4, X17
	ai	$3, $3, -1
	brnz	$3, L17
	TAIL	17
X17:
	stop	0x2017

	HEAD	18, 0, 0
	lnop
	.rept	8
	nop	$0
	.endr
	brnz	$4, X18
	ai	$3, $3, -1
	brnz	$3, L18
	TAIL	18
X18:
	stop	0x2018

	HEAD	19, 0, 0
	hbrr	B19, L19
	nop	$0
	ai	$3, $3, -1
B19:
	brnz	$3, L19
	TAIL	19

	HEAD	20, 0, 0
	hbrr	B20, L20
	.rept	10
	nop	$0
	.endr
	ai	$3, $3, -1
B20:
	brnz	$3, L20
	TAIL	20

	.macro	DUAL k, off
	HEAD	\k, \off, 1
	ai	$3, $3, -1
	lnop
	nop	$0
	lnop
	nop	$0
	lnop
	nop	$0
B\k:
	brnz	$3, L\k
	TAIL	\k
	.endm

	DUAL	21, 0
	DUAL	22, 4

	HEAD	23, 0, 1
	lqa	$5, 0x7F00
	a	$3, $3, $5
	ai	$3, $3, -1
B23:
	brnz	$3, L23
	TAIL	23

	HEAD	24, 0, 1
	rotqbyi	$3, $3, 0
	ai	$3, $3, -1
B24:
	brnz	$3, L24
	TAIL	24

	HEAD	25, 0, 1
	mpy	$5, $5, $9
	ai	$3, $3, -1
B25:
	brnz	$3, L25
	TAIL	25

	HEAD	26, 0, 1
	brnz	$4, X26
	ai	$3, $3, -1
B26:
	brnz	$3, L26
	TAIL	26
X26:
	stop	0x2026

	/* 27: does the fetch group after a branch that is not taken dual
	 * issue? */
	HEAD	27, 0, 1
	nop	$0
	brnz	$4, X27
	ai	$3, $3, -1
	lnop
	nop	$0
	lnop
	nop	$0
B27:
	brnz	$3, L27
	TAIL	27
X27:
	stop	0x2027

	/* 28: the latency from rchcnt to a branch on its count. */
	HEAD	28, 4, 1
	rchcnt	$10, $ch0
	brnz	$10, X28
	ai	$3, $3, -1
B28:
	brnz	$3, L28
	TAIL	28
X28:
	stop	0x2028

	/* 29: the latency from ceq to a branch on its result. */
	HEAD	29, 4, 1
	ceq	$22, $4, $9
	brnz	$22, X29
	ai	$3, $3, -1
B29:
	brnz	$3, L29
	TAIL	29
X29:
	stop	0x2029

	/* 30-33: the poll loop of case 9, changed one way each. */
	.macro	POLLHEAD k
	HEAD	\k, 0x28, 0
	il	$9, 0
	ilhu	$8, 1
	br	P\k
	.org	0x400 + (\k) * 0x200 + 0x168
P\k:
	.endm

	POLLHEAD 30
	ceq	$22, $9, $8
	brnz	$22, X30
	ai	$9, $9, 1
	lqa	$10, 0x7F00
	nop	$127
	brz	$10, P30
X30:
	TAIL	30

	POLLHEAD 31
	ceq	$22, $9, $8
	brnz	$22, X31
	ai	$9, $9, 1
	lnop
	nop	$127
	rchcnt	$10, $ch0
	nop	$127
	brz	$10, P31
X31:
	TAIL	31

	POLLHEAD 32
	nop	$127
	rchcnt	$10, $ch0
	ceq	$22, $9, $8
	brnz	$22, X32
	ai	$9, $9, 1
	lnop
	nop	$127
	brz	$10, P32
X32:
	TAIL	32

	POLLHEAD 33
	nop	$127
	lqa	$10, 0x7F00
	ceq	$22, $9, $8
	brnz	$22, X33
	ai	$9, $9, 1
	lnop
	nop	$127
	brz	$10, P33
X33:
	TAIL	33

	/* 34, 35: a branch that is not taken in the first fetch group
	 * after a mispredicted loop branch, and in the second. */
	HEAD	34, 0, 0
	nop	$0
	brnz	$4, X34
	nop	$0
	lnop
	ai	$3, $3, -1
	lnop
	nop	$0
	brnz	$3, L34
	TAIL	34
X34:
	stop	0x2034

	HEAD	35, 0, 0
	nop	$0
	lnop
	nop	$0
	brnz	$4, X35
	ai	$3, $3, -1
	lnop
	nop	$0
	brnz	$3, L35
	TAIL	35
X35:
	stop	0x2035

	/* 36-41: case 1's loop with its target in the upper half of a
	 * 128-byte line. */
	LOOP3	36, 0x40, 0
	LOOP3	37, 0x48, 0
	LOOP3	38, 0x60, 0
	LOOP3	39, 0x68, 0
	LOOP3	40, 0x70, 0
	LOOP3	41, 0x7C, 0

	/* 42: the poll loop of case 9 at 0x28 modulo 128. */
	HEAD	42, 0x28, 0
	il	$9, 0
	ilhu	$8, 1
	br	P42
	.org	0x400 + 42 * 0x200 + 0x1A8
P42:
	ceq	$22, $9, $8
	brnz	$22, X42
	ai	$9, $9, 1
	rchcnt	$10, $ch0
	nop	$127
	brz	$10, P42
X42:
	TAIL	42

	.org	0x400 + 43 * 0x200
	stop	0x2000

	.org	0x7F00
	.rept	4
	.long	0
	.endr

	.org	0x8000
slots:
	.rept	48 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
