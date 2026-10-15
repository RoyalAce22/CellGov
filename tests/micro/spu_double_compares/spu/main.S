/* SPU program for spu_double_compares, run on a raw SPU.
 *
 * Six entry points, 0x40 bytes apart from _start. Entry k loads A into
 * $4 and B into $5, runs one word with RT 3, RA 4 and RB 5, stores $3
 * to result slot k, and stops with signal 0x2000 + k:
 *
 *   0  dfceq   0x78614203      3  dfcmgt  0x59614203
 *   1  dfcmeq  0x79614203      4  dftsv   0x77FFC203 (I7 = 0x7F)
 *   2  dfcgt   0x58614203      5  0x90000000, an unassigned word
 *
 * A = (1.0, QNaN) and B = (1.0, 2.0), so each compare and the class
 * test gives a different answer in the two doubleword slots. Result
 * slot k sits at _start + 0x300 + 16k and starts as 0xEEEEEEEE words:
 * a slot the entry never stored to keeps them.
 */
	.text
	.global	_start
_start:

	.macro	ENTRY word, k
	.org	\k * 0x40
	lqr	$4, operand_a
	lqr	$5, operand_b
	.long	\word
	stqr	$3, result_\k
	stop	0x2000 + \k
	.endm

	ENTRY	0x78614203, 0
	ENTRY	0x79614203, 1
	ENTRY	0x58614203, 2
	ENTRY	0x59614203, 3
	ENTRY	0x77FFC203, 4
	ENTRY	0x90000000, 5

	.org	0x200
operand_a:
	.long	0x3FF00000, 0x00000000, 0x7FF80000, 0x00000000
operand_b:
	.long	0x3FF00000, 0x00000000, 0x40000000, 0x00000000

	.org	0x300
result_0:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
result_1:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
result_2:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
result_3:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
result_4:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
result_5:	.long	0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE, 0xEEEEEEEE
