/* SPU program for spu_snr_run_save.
 *
 * arg0 is the effective address of a 32-byte block. Report both
 * signal-notification registers into its first half, wait on the
 * inbound mailbox, report both again into its second half, and exit
 * the thread. A report is the channel count and, when the count is
 * nonzero, the value read (0xFFFFFFFF otherwise); no register read
 * waits.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned int out[4] __attribute__((aligned(16)));

static void report(unsigned int ea)
{
    out[0] = spu_readchcnt(SPU_RdSigNotify1);
    out[1] = out[0] ? spu_readch(SPU_RdSigNotify1) : 0xFFFFFFFF;
    out[2] = spu_readchcnt(SPU_RdSigNotify2);
    out[3] = out[2] ? spu_readch(SPU_RdSigNotify2) : 0xFFFFFFFF;

    mfc_put((void *)out, ea, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long arg_scratch_ea)
{
    report((unsigned int)arg_scratch_ea);
    (void)spu_readch(SPU_RdInMbox);
    report((unsigned int)arg_scratch_ea + 16);

    spu_thread_exit(0);
    return 0;
}
