/* SPU program: two edges of the user events an SPU sends through its
 * outbound interrupt mailbox.
 *
 * arg1 is the effective address of the result block (ppu/main.c lists
 * its layout).
 *
 * Phase 1 waits on SPU_RdSigNotify1. The PPU has put a message in the
 * inbound mailbox before it writes the signal. The program then sends
 * on OCCUPIED_PORT while that message waits, reads the inbound mailbox
 * once, and reads it a second time only if a word arrives within a
 * bounded spin.
 *
 * Phase 2 waits on SPU_RdSigNotify2. The PPU has destroyed the queue
 * connected to DESTROYED_PORT before it writes the signal. The program
 * sends and throws on that port.
 *
 * The mailbox words are written as spu/main.c of
 * spu_intr_mbox_to_event_queue writes them.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define DESTROYED_PORT 3
#define OCCUPIED_PORT  5
#define THROW_BIT      0x40000000u
#define SPIN           200000
#define TAG            1

static volatile unsigned int out[8] __attribute__((aligned(16)));

static unsigned int intr_word(unsigned int port, unsigned int data0)
{
    return (port << 24) | (data0 & 0x00FFFFFFu);
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned int i;
    (void)arg0;
    (void)arg2;
    (void)arg3;

    (void)spu_readch(SPU_RdSigNotify1);
    out[0] = spu_readchcnt(SPU_RdInMbox);
    spu_writech(SPU_WrOutMbox, 0x88888888u);
    spu_writech(SPU_WrOutIntrMbox, intr_word(OCCUPIED_PORT, 0x777777));
    out[1] = spu_readch(SPU_RdInMbox);
    for (i = 0; i < SPIN && spu_readchcnt(SPU_RdInMbox) == 0; i++)
        ;
    out[2] = spu_readchcnt(SPU_RdInMbox);
    out[3] = out[2] != 0 ? spu_readch(SPU_RdInMbox) : 0xEEEEEEEEu;

    (void)spu_readch(SPU_RdSigNotify2);
    spu_writech(SPU_WrOutMbox, 0x22222222u);
    spu_writech(SPU_WrOutIntrMbox, intr_word(DESTROYED_PORT, 0x111111));
    out[4] = spu_readch(SPU_RdInMbox);
    spu_writech(SPU_WrOutMbox, 0x44444444u);
    spu_writech(SPU_WrOutIntrMbox, THROW_BIT | intr_word(DESTROYED_PORT, 0x333333));
    for (i = 0; i < SPIN && spu_readchcnt(SPU_RdInMbox) == 0; i++)
        ;
    out[5] = spu_readchcnt(SPU_RdInMbox);
    out[6] = spu_readchcnt(SPU_WrOutIntrMbox);
    out[7] = 0;

    mfc_put((void *)out, result_ea + 16, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
