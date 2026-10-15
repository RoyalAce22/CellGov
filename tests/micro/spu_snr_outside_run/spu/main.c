/* SPU program for spu_snr_outside_run.
 *
 * arg0 is the effective address of a 16-byte block. Put there the
 * channel count of each signal-notification register and, when the
 * count is nonzero, the value read from it (0xFFFFFFFF otherwise),
 * then exit the thread. No read waits: a register is read only when
 * its count says it holds a value.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned int out[4] __attribute__((aligned(16)));

int main(unsigned long long arg_scratch_ea)
{
    out[0] = spu_readchcnt(SPU_RdSigNotify1);
    out[1] = out[0] ? spu_readch(SPU_RdSigNotify1) : 0xFFFFFFFF;
    out[2] = spu_readchcnt(SPU_RdSigNotify2);
    out[3] = out[2] ? spu_readch(SPU_RdSigNotify2) : 0xFFFFFFFF;

    mfc_put((void *)out, (unsigned int)arg_scratch_ea, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
