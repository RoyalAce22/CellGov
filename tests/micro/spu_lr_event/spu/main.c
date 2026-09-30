/* SPU program: the lock-line reservation lost event.
 *
 * arg1 is the effective address of a 512-byte block: a 32-byte result
 * at +0, lock line A at +128, lock line B at +256, and a ready flag at
 * +384. With Lr enabled the program:
 *   1. reserves A and stores it with putllc,
 *   2. reserves A, then reserves B,
 *   3. stores B with putlluc,
 * and reads the SPU_RdEventStat count after each. None of these is a
 * store by an outside entity. It then reserves A, sets the ready flag,
 * and reads SPU_RdEventStat, which waits until the PPU stores into A.
 *
 * Result layout (32 bytes):
 *   +0:  u32 status (0), u32 count after 1, u32 count after 2,
 *        u32 count after 3
 *   +16: u32 putllc status, u32 event status read, u32 count after the
 *        acknowledgment, u32 first word of A after the PPU's store
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

static volatile unsigned int line[32] __attribute__((aligned(128)));
static volatile unsigned int out[8] __attribute__((aligned(16)));
static volatile unsigned int ready[4] __attribute__((aligned(16)));

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

static void reserve(unsigned int ea)
{
    mfc_getllar((void *)line, ea, 0, 0);
    mfc_read_atomic_status();
}

int main(unsigned long long arg0,
         unsigned long long arg_block_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    /* The PPU passes a 32-bit address; only the low word carries it. */
    unsigned int block = (unsigned int)arg_block_ea;
    unsigned int line_a = block + 128;
    unsigned int line_b = block + 256;
    unsigned int ev;
    (void)arg0;
    (void)arg2;
    (void)arg3;

    spu_writech(SPU_WrEventMask, MFC_LLR_LOST_EVENT);

    reserve(line_a);
    mfc_putllc((void *)line, line_a, 0, 0);
    out[4] = mfc_read_atomic_status();
    out[1] = spu_readchcnt(SPU_RdEventStat);

    reserve(line_a);
    reserve(line_b);
    out[2] = spu_readchcnt(SPU_RdEventStat);

    mfc_putlluc((void *)line, line_b, 0, 0);
    mfc_read_atomic_status();
    out[3] = spu_readchcnt(SPU_RdEventStat);

    reserve(line_a);
    ready[0] = 1;
    mfc_put((void *)ready, block + 384, 16, TAG, 0, 0);
    wait_tag();

    ev = spu_readch(SPU_RdEventStat);
    spu_writech(SPU_WrEventAck, ev);
    out[5] = ev;
    out[6] = spu_readchcnt(SPU_RdEventStat);

    reserve(line_a);
    out[7] = line[0];

    out[0] = 0;
    mfc_put((void *)out, block, 32, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
