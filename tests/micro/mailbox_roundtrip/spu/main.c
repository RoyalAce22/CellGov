/* SPU program: mailbox roundtrip.
 *
 * Reads one value from the inbound mailbox (PPU -> SPU), XORs it
 * with 0xFFFFFFFF (bitwise NOT), and DMA puts the result to the
 * shared result buffer in main memory.
 *
 * arg1 = EA of the 128-byte-aligned result buffer.
 *
 * TestResult layout:
 *   +0: u32 status (0 = pass)
 *   +4: u32 value  (the echoed/transformed mailbox value)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define RESULT_LS_ADDR  0x3000
#define DMA_TAG         0

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int ea_lo = (unsigned int)argp;

    /* Read the command from the inbound mailbox (blocks until PPU writes). */
    unsigned int command = spu_readch(SPU_RdInMbox);

    /* Transform: bitwise NOT. */
    unsigned int response = command ^ 0xFFFFFFFF;

    /* Write result to LS. */
    volatile unsigned int *result = (volatile unsigned int *)RESULT_LS_ADDR;
    result[0] = 0;          /* status = pass */
    result[1] = response;   /* transformed value */
    result[2] = 0;
    result[3] = 0;

    /* DMA put to main memory. */
    mfc_put((void *)RESULT_LS_ADDR, ea_lo, 16, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
