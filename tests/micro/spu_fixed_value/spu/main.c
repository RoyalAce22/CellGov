/* SPU program: write a fixed known value to main memory via DMA.
 *
 * Receives the EA (effective address) of a 16-byte-aligned result
 * buffer in arg0 (SPU thread argument, read from channel 3).
 * Writes 0x1337BAAD as the value field of a TestResult struct,
 * DMA puts it to main memory, waits for completion, then exits.
 *
 * TestResult layout (8 bytes, big-endian):
 *   +0: u32 status   (0 = pass)
 *   +4: u32 value    (0x1337BAAD)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define RESULT_LS_ADDR  0x3000
#define FIXED_VALUE     0x1337BAAD
#define DMA_TAG         0

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    /* argp is the EA of the result buffer in main memory. */
    unsigned int ea_lo = (unsigned int)argp;

    /* Build the TestResult in LS at a 16-byte aligned address.
     * MFC DMA requires 16-byte alignment for the LS address. */
    volatile unsigned int *result = (volatile unsigned int *)RESULT_LS_ADDR;
    result[0] = 0;             /* status = 0 (pass) */
    result[1] = FIXED_VALUE;   /* value  = 0x1337BAAD */
    result[2] = 0;             /* padding to 16 bytes */
    result[3] = 0;

    /* DMA put: LS 0x3000 -> EA argp, 16 bytes (minimum aligned size). */
    mfc_put((void *)RESULT_LS_ADDR, ea_lo, 16, DMA_TAG, 0, 0);

    /* Wait for DMA completion. */
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
