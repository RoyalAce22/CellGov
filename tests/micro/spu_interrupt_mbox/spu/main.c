/* SPU program: an inbound-mailbox interrupt handler.
 *
 * arg1 is the effective address of a 512-byte block: a 32-byte result
 * at +0 and a ready flag at +384. The program installs a branch to its
 * handler at local-store address 0, enables the inbound mailbox event,
 * turns interrupts on, sets the ready flag, and spins until the handler
 * has run. The PPU then writes one mailbox message.
 *
 * The handler reads SPU_RdEventStat, the message, SPU_RdMachStat and
 * SRR0, acknowledges the event, and returns with irete.
 *
 * Result layout (32 bytes):
 *   +0:  u32 status (0), u32 message, u32 event status the handler read,
 *        u32 machine status inside the handler
 *   +16: u32 machine status after the return, u32 inbound mailbox count
 *        after the return, u32 SRR0 is non-zero, u32 0
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1

volatile unsigned int cgov_irq_save[4] __attribute__((aligned(16)));
volatile unsigned int cgov_irq_events[4] __attribute__((aligned(16)));
volatile unsigned int cgov_irq_message[4] __attribute__((aligned(16)));
volatile unsigned int cgov_irq_machstat[4] __attribute__((aligned(16)));
volatile unsigned int cgov_irq_srr0[4] __attribute__((aligned(16)));
volatile unsigned int cgov_irq_done[4] __attribute__((aligned(16)));

static volatile unsigned int out[8] __attribute__((aligned(16)));
static volatile unsigned int ready[4] __attribute__((aligned(16)));

/* Channel 0 is SPU_RdEventStat, 2 SPU_WrEventAck, 13 SPU_RdMachStat,
 * 15 SPU_RdSRR0 and 29 SPU_RdInMbox. 0x10 is the Mb event bit. */
__asm__(
    ".text\n"
    ".align 3\n"
    ".global cgov_irq_handler\n"
    "cgov_irq_handler:\n"
    "  stqa $2, cgov_irq_save\n"
    "  rdch $2, $ch0\n"
    "  stqa $2, cgov_irq_events\n"
    "  rdch $2, $ch29\n"
    "  stqa $2, cgov_irq_message\n"
    "  rdch $2, $ch13\n"
    "  stqa $2, cgov_irq_machstat\n"
    "  rdch $2, $ch15\n"
    "  stqa $2, cgov_irq_srr0\n"
    "  il $2, 16\n"
    "  wrch $ch2, $2\n"
    "  il $2, 1\n"
    "  stqa $2, cgov_irq_done\n"
    "  lqa $2, cgov_irq_save\n"
    "  irete\n");

extern void cgov_irq_handler(void);

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long arg0,
         unsigned long long arg_block_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    /* The PPU passes a 32-bit address; only the low word carries it. */
    unsigned int block = (unsigned int)arg_block_ea;
    unsigned int target = (unsigned int)cgov_irq_handler;
    /* bra handler, then three nops. */
    vec_uint4 entry = { 0x30000000u | (((target >> 2) & 0xFFFFu) << 7),
                        0x40200000u, 0x40200000u, 0x40200000u };
    (void)arg0;
    (void)arg2;
    (void)arg3;

    __asm__ volatile("stqa %0, 0" : : "r"(entry) : "memory");
    spu_sync();

    spu_writech(SPU_WrEventMask, MFC_IN_MBOX_AVAILABLE_EVENT);
    spu_ienable();

    ready[0] = 1;
    mfc_put((void *)ready, block + 384, 16, TAG, 0, 0);
    wait_tag();

    while (cgov_irq_done[0] == 0) {
    }

    out[1] = cgov_irq_message[0];
    out[2] = cgov_irq_events[0];
    out[3] = cgov_irq_machstat[0];
    out[4] = spu_readch(SPU_RdMachStat);
    out[5] = spu_readchcnt(SPU_RdInMbox);
    out[6] = cgov_irq_srr0[0] != 0;
    out[7] = 0;
    out[0] = 0;
    mfc_put((void *)out, block, 32, TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
