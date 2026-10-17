/* SPU program for spu_invalid_npc, run on a raw SPU.
 *
 * Seventy-five cases, 0x40 bytes apart from _start: fifteen invalid
 * words, each in five arrangements. Arrangements 0 to 3 start with
 * `il $6, 0` and `il $8, 0x77`, then place a pair at +0x08 and +0x0C:
 *
 *   arrangement 0  +0x08 the word          +0x0C stqr $8 to mark k (odd pipe)
 *   arrangement 1  +0x08 the word          +0x0C ai $6, $6, 1      (even pipe)
 *   arrangement 2  +0x08 ai $6, $6, 1      +0x0C the word
 *   arrangement 3  +0x08 stqr $8 to mark k +0x0C the word
 *
 * Arrangement 4 is the entry shape of the spu_double_compares probe:
 * `lqr $4` and `lqr $5` at +0x00 and +0x04, the word at +0x08 and
 * `stqr $8` to mark k at +0x0C.
 *
 * Every case stops with signal 0x2000 + k at +0x10. Case k = 5 * w + a,
 * for word w and arrangement a. Mark k is 16 bytes at 0x2000 + 16k and
 * starts as zero words; a stqr that ran fills it with 0x77.
 *
 * The routine at 0x1800 stores $6 to the dump slot at 0x1900 and stops
 * with 0x3FFE. The PPU runs it after each case to read whether that
 * case's `ai` ran.
 */
	.text
	.global	_start
_start:

	.macro	CASE word, k, arr
	.org	\k * 0x40
	.if	\arr == 4
	lqr	$4, dump_slot
	lqr	$5, dump_slot
	.long	\word
	stqr	$8, marks+\k*16
	.else
	il	$6, 0
	il	$8, 0x77
	.endif
	.if	\arr == 0
	.long	\word
	stqr	$8, marks+\k*16
	.endif
	.if	\arr == 1
	.long	\word
	ai	$6, $6, 1
	.endif
	.if	\arr == 2
	ai	$6, $6, 1
	.long	\word
	.endif
	.if	\arr == 3
	stqr	$8, marks+\k*16
	.long	\word
	.endif
	stop	0x2000 + \k
	.endm

	.macro	INVALID word, w
	CASE	\word, (\w*5+0), 0
	CASE	\word, (\w*5+1), 1
	CASE	\word, (\w*5+2), 2
	CASE	\word, (\w*5+3), 3
	CASE	\word, (\w*5+4), 4
	.endm

	INVALID	0x78614203, 0	/* dfceq */
	INVALID	0x79614203, 1	/* dfcmeq */
	INVALID	0x58614203, 2	/* dfcgt */
	INVALID	0x59614203, 3	/* dfcmgt */
	INVALID	0x77FFC203, 4	/* dftsv */
	INVALID	0x90000000, 5	/* RRR opcode 1001: unassigned */
	INVALID	0xA0000000, 6	/* RRR opcode 1010: unassigned */
	INVALID	0x00800000, 7	/* 11-bit 00000000100: unassigned */
	INVALID	0x01000000, 8	/* 11-bit 00000001000: unassigned */
	INVALID	0x28200000, 9	/* 11-bit 00101000001: unassigned */
	INVALID	0x3B400000, 10	/* 11-bit 00111011010: unassigned */
	INVALID	0x3E200000, 11	/* 11-bit 00111110001: unassigned */
	INVALID	0x50000000, 12	/* 11-bit 01010000000: unassigned */
	INVALID	0x60000000, 13	/* 11-bit 01100000000: unassigned */
	INVALID	0x70000000, 14	/* 11-bit 01110000000: unassigned */

	.org	0x1800
	stqr	$6, dump_slot
	stop	0x3FFE

	.org	0x1900
dump_slot:
	.long	0, 0, 0, 0

	.org	0x2000
marks:
	.space	75 * 16
