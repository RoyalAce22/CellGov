/* SPU program: the PPE overruns the SPU inbound mailbox.
 *
 * arg1 is the effective address of a 32-byte result block, arg2 the
 * effective address of a 16-byte flag the PPU sets once it has written
 * five messages into this SPU's four-entry inbound mailbox.
 *
 * Waits for the flag, reads the mailbox count, reads four messages,
 * reads the count again, and writes
 *   +0:  u32 status (0), u32 count before, u32 count after, u32 0
 *   +16: u32 the four messages in the order read
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned int flag[4] __attribute__((aligned(16)));
static volatile unsigned int result[8] __attribute__((aligned(16)));

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg_flag_ea,
         unsigned long long arg3)
{
    /* The PPU passes 32-bit addresses; only the low word of each
     * argument carries one. */
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned int flag_ea = (unsigned int)arg_flag_ea;
    int i;
    (void)arg0;
    (void)arg3;

    do {
        mfc_get((void *)flag, flag_ea, 16, TAG, 0, 0);
        wait_tag();
    } while (flag[0] == 0);

    result[0] = 0;
    result[1] = spu_readchcnt(SPU_RdInMbox);
    for (i = 0; i < 4; i++)
        result[4 + i] = spu_readch(SPU_RdInMbox);
    result[2] = spu_readchcnt(SPU_RdInMbox);
    result[3] = 0;

    mfc_put((void *)result, result_ea, 32, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
