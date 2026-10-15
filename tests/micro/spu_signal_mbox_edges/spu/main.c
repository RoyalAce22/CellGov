/* SPU program for spu_signal_mbox_edges.
 *
 * Read the inbound mailbox, which waits until the PPU writes it, then
 * exit the thread with the value read.
 */

#include <spu_intrinsics.h>
#include <sys/spu_thread.h>

int main(void)
{
    unsigned int value = spu_readch(SPU_RdInMbox);

    spu_thread_exit(value);
    return 0;
}
