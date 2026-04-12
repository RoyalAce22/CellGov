/* SPU program: barrier/wakeup ordering.
 *
 * Shared binary for two SPU threads in one group.  The thread index
 * is encoded in the low byte of arg1 (the base EA is 256-byte
 * aligned, so low 8 bits are free).
 *
 * SPU 0: writes a flag to shared memory, then writes its result.
 * SPU 1: polls shared memory until SPU 0's flag is visible, then
 *         writes its own result.  This proves inter-SPU ordering
 *         through main memory.
 *
 * arg1 = (EA of 256-byte buffer) | thread_index
 *
 * Buffer layout:
 *   [0..16]    SPU 0 result (status + marker)
 *   [16..32]   SPU 1 result (status + marker)
 *   [128..144] flag area (SPU 0 writes, SPU 1 polls)
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define LS_RESULT    0x3000
#define LS_FLAG      0x3100
#define RESULT0_OFF  0
#define RESULT1_OFF  16
#define FLAG_OFF     128
#define DMA_TAG      0
#define MAX_POLLS    100000
#define FLAG_VALUE   0xAAAA0000u
#define SPU0_MARKER  0xAAAA0000u
#define SPU1_MARKER  0xBBBB0001u

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int raw = (unsigned int)argp;
    unsigned int idx = raw & 0xFFu;
    unsigned int base_ea = raw & ~0xFFu;
    volatile unsigned int *result = (volatile unsigned int *)LS_RESULT;
    volatile unsigned int *flag = (volatile unsigned int *)LS_FLAG;

    if (idx == 0) {
        /* SPU 0: write flag, then write result. */

        flag[0] = FLAG_VALUE;
        flag[1] = 0;
        flag[2] = 0;
        flag[3] = 0;

        mfc_put((void *)LS_FLAG, base_ea + FLAG_OFF, 16, DMA_TAG, 0, 0);
        mfc_write_tag_mask(1 << DMA_TAG);
        mfc_read_tag_status_all();

        result[0] = 0;
        result[1] = SPU0_MARKER;
        result[2] = 0;
        result[3] = 0;

        mfc_put((void *)LS_RESULT, base_ea + RESULT0_OFF, 16, DMA_TAG, 0, 0);
        mfc_write_tag_mask(1 << DMA_TAG);
        mfc_read_tag_status_all();

    } else {
        /* SPU 1: poll until SPU 0's flag is visible. */

        int polls;
        flag[0] = 0;
        flag[1] = 0;
        flag[2] = 0;
        flag[3] = 0;

        for (polls = 0; polls < MAX_POLLS; polls++) {
            mfc_get((void *)LS_FLAG, base_ea + FLAG_OFF, 16, DMA_TAG, 0, 0);
            mfc_write_tag_mask(1 << DMA_TAG);
            mfc_read_tag_status_all();

            if (flag[0] == FLAG_VALUE)
                break;
        }

        if (flag[0] != FLAG_VALUE) {
            /* Timeout: SPU 0 flag never seen. */
            result[0] = 1;
            result[1] = 0;
        } else {
            /* Flag seen: SPU 0 completed before us. */
            result[0] = 0;
            result[1] = SPU1_MARKER;
        }
        result[2] = 0;
        result[3] = 0;

        mfc_put((void *)LS_RESULT, base_ea + RESULT1_OFF, 16, DMA_TAG, 0, 0);
        mfc_write_tag_mask(1 << DMA_TAG);
        mfc_read_tag_status_all();
    }

    spu_thread_exit(0);
    return 0;
}
