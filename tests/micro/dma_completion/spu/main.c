/* SPU program: DMA put completion visibility.
 *
 * Fills 128 bytes of LS with a repeating pattern (0xDE, 0xAD, 0xBE, 0xEF),
 * DMA puts the full buffer to main memory, waits for completion, then
 * writes status=0 to the first 16 bytes and DMA puts that too.
 *
 * The PPU verifies all 128 bytes arrived correctly.
 *
 * arg1 = EA of the 128-byte-aligned result buffer.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define PATTERN_LS_ADDR 0x3100
#define STATUS_LS_ADDR  0x3000
#define PATTERN_SIZE    128
#define DMA_TAG         0

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int ea_lo = (unsigned int)argp;
    unsigned char *pattern = (unsigned char *)PATTERN_LS_ADDR;
    int i;

    /* Fill LS with a repeating 4-byte pattern. */
    for (i = 0; i < PATTERN_SIZE; i++)
    {
        unsigned char seq[4] = { 0xDE, 0xAD, 0xBE, 0xEF };
        pattern[i] = seq[i & 3];
    }

    /* DMA put the 128-byte pattern to EA + 16 (after the status header). */
    mfc_put((void *)PATTERN_LS_ADDR, ea_lo + 16, PATTERN_SIZE, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    /* Write status=0 to the first 16 bytes. */
    volatile unsigned int *status = (volatile unsigned int *)STATUS_LS_ADDR;
    status[0] = 0;     /* status = pass */
    status[1] = PATTERN_SIZE;
    status[2] = 0;
    status[3] = 0;

    mfc_put((void *)STATUS_LS_ADDR, ea_lo, 16, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
