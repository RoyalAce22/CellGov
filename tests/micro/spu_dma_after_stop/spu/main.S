/* SPU program for spu_dma_after_stop, run once on a raw SPU.
 *
 * The PPU writes the effective address of a 64 KB buffer of zeros to
 * the word at 0x1F00 before the run. The program queues four puts of
 * the 16 KB at 0x4000 (0x5A5A5A5A words) to the buffer, one per 16 KB
 * of it, all under tag 1, and stops with 0x2000 at once, without
 * waiting for the tag.
 */
	.text
	.global	_start
_start:
	lqa	$11, 0x1F00
	il	$9, 0
	wrch	$ch17, $9
	il	$4, 0x4000
	ila	$5, 0x4000
	il	$15, 1
	il	$6, 0x20

	.macro	PUT k
	ila	$19, \k*0x4000
	a	$12, $11, $19
	wrch	$ch16, $4
	wrch	$ch18, $12
	wrch	$ch19, $5
	wrch	$ch20, $15
	wrch	$ch21, $6
	.endm

	PUT	0
	PUT	1
	PUT	2
	PUT	3
	stop	0x2000

	.org	0x4000
data:
	.rept	0x4000 / 4
	.long	0x5A5A5A5A
	.endr
