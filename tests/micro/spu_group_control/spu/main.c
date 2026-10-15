/* SPU program for spu_group_control.
 *
 * arg0 is the mode, arg1 the effective address of the result block
 * (ppu/main.c lists its layout).
 *
 * MODE_PARK: read SPU_RdSigNotify1, which nothing ever writes, so the
 * thread waits until the PPU terminates its group.
 *
 * MODE_ARGS: put arg2 and arg3 at +96, then exit with EXIT_STATUS.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define MODE_PARK 1
#define MODE_ARGS 2

#define EXIT_STATUS 0x00C0FFEE
#define TAG         1

static volatile unsigned long long out[2] __attribute__((aligned(16)));

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    if ((unsigned int)arg0 == MODE_PARK) {
        (void)spu_readch(SPU_RdSigNotify1);
        spu_thread_exit(0);
        return 0;
    }

    out[0] = arg2;
    out[1] = arg3;
    mfc_put((void *)out, (unsigned int)arg_result_ea + 96, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(EXIT_STATUS);
    return 0;
}
