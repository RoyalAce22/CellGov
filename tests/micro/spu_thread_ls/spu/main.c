/* SPU program for spu_thread_ls.
 *
 * Fills 32 bytes of local store at BUF_LSA with 0x00..0x1F, puts a
 * nonzero ready word to the PPU, then waits on its inbound mailbox
 * while the PPU writes and reads that buffer with
 * sys_spu_thread_write_ls and sys_spu_thread_read_ls. Once a message
 * arrives it puts the 32 bytes to the PPU and exits.
 *
 * arg0 = 1: exit at once with status 7 instead.
 * arg1 = EA of a 128-byte-aligned PPU buffer: the ready word at +0,
 *        the 32 bytes at +16.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define BUF_LSA    0x3000
#define STATUS_LSA 0x3100
#define DMA_TAG    0

static void put_and_wait(unsigned int lsa, unsigned int ea, unsigned int size)
{
    mfc_put((void *)lsa, ea, size, DMA_TAG, 0, 0);
    mfc_write_tag_mask(1 << DMA_TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long arg0,
         unsigned long long arg1,
         unsigned long long arg2)
{
    unsigned int ea = (unsigned int)arg1;
    volatile unsigned char *buf = (volatile unsigned char *)BUF_LSA;
    volatile unsigned int *status = (volatile unsigned int *)STATUS_LSA;
    int i;

    (void)arg2;
    if (arg0 == 1) {
        spu_thread_exit(7);
        return 0;
    }
    for (i = 0; i < 32; i++)
        buf[i] = (unsigned char)i;
    status[0] = 1;
    status[1] = 0;
    status[2] = 0;
    status[3] = 0;
    put_and_wait(STATUS_LSA, ea, 16);

    (void)spu_readch(SPU_RdInMbox);
    put_and_wait(BUF_LSA, ea + 16, 32);
    spu_thread_exit(0);
    return 0;
}
