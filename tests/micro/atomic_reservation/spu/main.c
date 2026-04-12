/* SPU program: atomic reservation (getllar / putllc).
 *
 * Loads 128 bytes from main memory using getllar (atomic load with
 * reservation), overwrites the LS copy with 0xBBBBBBBB, then stores
 * back using putllc (conditional store).  No contention exists, so
 * putllc should succeed on the first attempt.
 *
 * arg1 = EA of the 512-byte result buffer.
 *         [0..16]    status header
 *         [16..144]  data copy after atomic store
 *         [256..384] atomic target (128 bytes, 128-byte aligned)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define LS_ATOMIC   0x3100  /* 128-byte aligned LS addr for atomic ops */
#define LS_STATUS   0x3000  /* 16-byte status area */
#define DATA_OFFSET 256     /* offset of atomic target in main buffer */
#define COPY_OFFSET 16      /* offset for data copy in result */
#define DATA_SIZE   128
#define DMA_TAG     0

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int base_ea = (unsigned int)argp;
    unsigned int atomic_ea = base_ea + DATA_OFFSET;
    volatile unsigned int *data = (volatile unsigned int *)LS_ATOMIC;
    volatile unsigned int *status = (volatile unsigned int *)LS_STATUS;
    unsigned int put_stat;
    int i;

    /* Step 1: Load 128-byte cache line with reservation. */
    mfc_getllar((void *)LS_ATOMIC, (unsigned long long)atomic_ea, 0, 0);
    spu_readch(MFC_RdAtomicStat);

    /* Step 2: Overwrite LS copy with known pattern. */
    for (i = 0; i < 32; i++)
        data[i] = 0xBBBBBBBB;

    /* Step 3: Conditional store back to main memory. */
    mfc_putllc((void *)LS_ATOMIC, (unsigned long long)atomic_ea, 0, 0);
    put_stat = spu_readch(MFC_RdAtomicStat);

    /* Step 4: Build status header.
     *   [0] = 0 if putllc succeeded, 1 if failed
     *   [1] = raw MFC_RdAtomicStat after putllc (0 = success)
     */
    status[0] = (put_stat == 0) ? 0 : 1;
    status[1] = put_stat;
    status[2] = 0;
    status[3] = 0;

    /* Step 5: DMA put status header to result buffer. */
    mfc_put((void *)LS_STATUS, base_ea, 16, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    /* Step 6: DMA put data copy to result buffer + COPY_OFFSET. */
    mfc_put((void *)LS_ATOMIC, base_ea + COPY_OFFSET, DATA_SIZE, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
