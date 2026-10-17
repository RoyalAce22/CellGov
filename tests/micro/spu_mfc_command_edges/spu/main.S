/* SPU program for spu_mfc_command_edges, run once on a raw SPU.
 *
 * The PPU writes the effective address of a 4 KB buffer of zeros to the
 * word at 0x1F00 before the run. The program stores to eight result
 * slots of 16 bytes at 0x2000, which start as 0xEEEEEEEE words:
 *
 *   1. Put 16 bytes from 0x3000 (0x1111 words) to the buffer, tag 1,
 *      with every parameter written; wait for all tags; slot 0 is the
 *      tag status.
 *   2. Store 0x2222 words to 0x3000, write only MFC_Cmd = put, and wait
 *      for all tags; slot 1 is the tag status. The buffer's first word
 *      says what the second put wrote.
 *   3. Write 5 to MFC_WrListStallAck with no list stalled; slot 2 is
 *      the MFC_RdListStallStat count. Then get 16 bytes from the buffer
 *      to 0x3100 with tag 5 and wait for all tags; slot 3 is the tag
 *      status.
 *   4. Write 3, a reserved value, to MFC_WrTagUpdate; slot 4 is the
 *      MFC_RdTagStat count, and slot 5 the tag status when the count
 *      is not 0. Then wait for all tags again; slot 6 is the tag
 *      status.
 *
 * The SPU stops with 0x2000.
 */
	.text
	.global	_start
_start:
	lqa	$11, 0x1F00
	il	$9, 0
	wrch	$ch17, $9
	il	$15, 1
	il	$16, -1
	il	$17, 2

	/* Wait for every tag group; the tag status goes to \slot. */
	.macro	WAIT_ALL slot
	wrch	$ch22, $16
	wrch	$ch23, $17
	rdch	$25, $ch24
	stqa	$25, \slot
	.endm

	il	$3, 0x1111
	stqa	$3, 0x3000
	il	$4, 0x3000
	wrch	$ch16, $4
	wrch	$ch18, $11
	il	$5, 16
	wrch	$ch19, $5
	wrch	$ch20, $15
	il	$6, 0x20
	wrch	$ch21, $6
	WAIT_ALL slots
	il	$3, 0x2222
	stqa	$3, 0x3000
	wrch	$ch21, $6
	WAIT_ALL slots + 0x10

	il	$7, 5
	wrch	$ch26, $7
	rchcnt	$8, $ch25
	stqa	$8, slots + 0x20
	il	$4, 0x3100
	wrch	$ch16, $4
	wrch	$ch18, $11
	wrch	$ch19, $5
	wrch	$ch20, $7
	il	$6, 0x40
	wrch	$ch21, $6
	WAIT_ALL slots + 0x30

	il	$10, 3
	wrch	$ch23, $10
	rchcnt	$12, $ch24
	stqa	$12, slots + 0x40
	brz	$12, counted
	rdch	$13, $ch24
	stqa	$13, slots + 0x50
counted:
	WAIT_ALL slots + 0x60
	stop	0x2000

	.org	0x2000
slots:
	.rept	8 * 0x10 / 4
	.long	0xEEEEEEEE
	.endr
