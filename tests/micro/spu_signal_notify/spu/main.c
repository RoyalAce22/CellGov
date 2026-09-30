/* SPU program: read both signal-notification registers after the PPU
 * wrote them with sys_spu_thread_write_snr.
 *
 * arg1 is the effective address of a 48-byte result block, arg2 the
 * effective address of a 16-byte flag the PPU sets after its writes.
 * The PPU fills the block's first 16 bytes itself; this program writes
 *   +16: u32 count 1, u32 count 2, u32 register 1, u32 register 2
 *   +32: u32 count 1 after, u32 count 2 after, and two words the PPU
 *        fills after join
 * and a status of 0 at +0.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned int flag[4] __attribute__((aligned(16)));
static volatile unsigned int result[8] __attribute__((aligned(16)));
static volatile unsigned int status[4] __attribute__((aligned(16)));

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
    (void)arg0;
    (void)arg3;

    do {
        mfc_get((void *)flag, flag_ea, 16, TAG, 0, 0);
        wait_tag();
    } while (flag[0] == 0);

    result[0] = spu_readchcnt(SPU_RdSigNotify1);
    result[1] = spu_readchcnt(SPU_RdSigNotify2);
    result[2] = spu_readch(SPU_RdSigNotify1);
    result[3] = spu_readch(SPU_RdSigNotify2);
    result[4] = spu_readchcnt(SPU_RdSigNotify1);
    result[5] = spu_readchcnt(SPU_RdSigNotify2);
    result[6] = 0;
    result[7] = 0;
    mfc_put((void *)result, result_ea + 16, 32, TAG, 0, 0);
    wait_tag();

    status[0] = 0;
    mfc_put((void *)status, result_ea, 4, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
