/* SPU program for spu_dma_after_exit, run as the one thread of an SPU
 * thread group.
 *
 * The thread's first argument is the effective address of a 256 KB
 * buffer of zeros. The program queues sixteen puts of the 16 KB at
 * 0x4000 (0x5A5A5A5A words) to the buffer, one per 16 KB of it, all
 * under tag 1, which fills the MFC queue, and exits the thread with
 * status 0 at once, without waiting for the tag.
 */
	.text
	.global	_start
_start:
	wrch	$ch17, $3
	rotqbyi	$11, $3, 4
	il	$4, 0x4000
	ila	$5, 0x4000
	il	$15, 1
	il	$6, 0x20

	.macro	PUT k
	ila	$19, (\k)*0x4000
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
	PUT	4
	PUT	5
	PUT	6
	PUT	7
	PUT	8
	PUT	9
	PUT	10
	PUT	11
	PUT	12
	PUT	13
	PUT	14
	PUT	15
	il	$2, 0
	wrch	$ch28, $2
	stop	0x102

	.org	0x4000
data:
	.rept	0x4000 / 4
	.long	0x5A5A5A5A
	.endr
