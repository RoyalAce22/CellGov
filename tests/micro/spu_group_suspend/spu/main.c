/* SPU program for spu_group_suspend. arg0 picks what the thread does:
 *   0: read SPU_RdSigNotify1, which nothing ever writes, so the thread
 *      waits until the PPU terminates its group.
 *   1: read SPU_RdInMbox and exit with the message as its exit status.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

int main(unsigned long long arg0,
         unsigned long long arg1,
         unsigned long long arg2,
         unsigned long long arg3)
{
    (void)arg1;
    (void)arg2;
    (void)arg3;
    if (arg0 == 1) {
        unsigned int message = spu_readch(SPU_RdInMbox);
        spu_thread_exit((int)message);
        return 0;
    }
    (void)spu_readch(SPU_RdSigNotify1);
    spu_thread_exit(0);
    return 0;
}
