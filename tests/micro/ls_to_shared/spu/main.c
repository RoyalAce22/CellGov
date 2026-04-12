/* SPU program: local-store to shared-memory publication.
 *
 * Computes a 32-word chain in local store where each value depends on
 * the previous (proving LS store-to-load forwarding), then DMA puts
 * the result to main memory.
 *
 *   data[0] = 0xC0DE0000
 *   data[i] = data[i-1] + 0x01010101   (i = 1..31)
 *
 * arg1 = EA of the 256-byte result buffer.
 *         [0..16]    status header
 *         [16..144]  computed data (32 words, 128 bytes)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DATA_LS_ADDR   0x3100
#define STATUS_LS_ADDR 0x3000
#define DATA_SIZE      128
#define WORD_COUNT     32
#define SEED           0xC0DE0000u
#define STRIDE         0x01010101u
#define DMA_TAG        0

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int ea_lo = (unsigned int)argp;
    volatile unsigned int *data = (volatile unsigned int *)DATA_LS_ADDR;
    volatile unsigned int *status = (volatile unsigned int *)STATUS_LS_ADDR;
    int i;

    /* Compute chain: each word depends on the previous LS read. */
    data[0] = SEED;
    for (i = 1; i < WORD_COUNT; i++)
        data[i] = data[i - 1] + STRIDE;

    /* DMA put computed data to EA + 16. */
    mfc_put((void *)DATA_LS_ADDR, ea_lo + 16, DATA_SIZE, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    /* Write status header: pass. */
    status[0] = 0;          /* status = pass */
    status[1] = WORD_COUNT; /* number of values */
    status[2] = 0;
    status[3] = 0;

    /* DMA put status header to EA + 0. */
    mfc_put((void *)STATUS_LS_ADDR, ea_lo, 16, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
