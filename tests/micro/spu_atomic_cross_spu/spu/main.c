/* SPU program: atomic-contention participant.
 *
 * Each instance runs INCREMENTS_PER_THREAD CAS-retry increments on a
 * shared 128-byte cache line in main memory. The line's first word
 * is the counter. Retries are counted locally and written back to
 * each SPU's own result slot.
 *
 * arg1 = EA of the shared atomic line (128-byte aligned).
 * arg2 = EA of this SPU's 16-byte result slot.
 *
 * Result slot layout:
 *   [0] u32 status         (0 = completed)
 *   [4] u32 final_counter  (counter value last seen by this SPU)
 *   [8] u32 retries        (total putllc failures across N rounds)
 *   [12] u32 spe_id        (informational)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define LS_ATOMIC 0x3100
#define LS_STATUS 0x3000
#define DMA_TAG   0

#define INCREMENTS_PER_THREAD 32

int main(unsigned long long spe_id,
         unsigned long long arg_atomic_ea,
         unsigned long long arg_result_ea,
         unsigned long long arg4)
{
    (void)arg4;
    unsigned int atomic_ea = (unsigned int)arg_atomic_ea;
    unsigned int result_ea = (unsigned int)arg_result_ea;
    volatile unsigned int *data = (volatile unsigned int *)LS_ATOMIC;
    volatile unsigned int *status = (volatile unsigned int *)LS_STATUS;
    unsigned int retries = 0;
    unsigned int put_stat;
    unsigned int final_counter = 0;
    int i;

    for (i = 0; i < INCREMENTS_PER_THREAD; i++) {
        for (;;) {
            mfc_getllar((void *)LS_ATOMIC, (unsigned long long)atomic_ea, 0, 0);
            spu_readch(MFC_RdAtomicStat);
            unsigned int v = data[0];
            data[0] = v + 1;
            mfc_putllc((void *)LS_ATOMIC, (unsigned long long)atomic_ea, 0, 0);
            put_stat = spu_readch(MFC_RdAtomicStat);
            if (put_stat == 0) {
                final_counter = v + 1;
                break;
            }
            retries++;
        }
    }

    status[0] = 0;
    status[1] = final_counter;
    status[2] = retries;
    status[3] = (unsigned int)spe_id;

    mfc_put((void *)LS_STATUS, result_ea, 16, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
