/* SPU program for spu_mailbox_events: whether the Le and Me events
 * follow the outbound mailbox counts, and what the thread sees at start.
 *
 * arg1 is the effective address of the result block (ppu/main.c lists
 * its layout). The program writes 16 words at +16:
 *
 *   phase 0, at start: the event mask the thread starts with, the
 *     SPU_RdEventStat count with only Le enabled, the count with only Me
 *     enabled, and the status read with both enabled (0 when the count
 *     is 0)
 *   phase 1, Le and Me enabled: the status after SPU_WrOutMbox alone
 *     (0 when no event came within POLLS), the answer to a send on
 *     PORT, the status once the answer came, and the SPU_WrOutMbox
 *     count in the high half and the SPU_WrOutIntrMbox count in the low
 *   phase 2, Me alone enabled: the status after a throw on PORT, the
 *     count once Le is enabled too, the status read then (0 when the
 *     count is 0), and the SPU_WrOutMbox count Le was enabled at. LV2
 *     can read SPU_WrOutMbox after the program sees Me, so the program
 *     waits at most POLLS for that count before it enables Le.
 *   phase 3, A alone enabled: the count and the status (0 when the
 *     count is 0) at start, before phase 0, and again after phase 2.
 *     Only privileged software raises A, by a write to SPU_PrivCntl.
 *
 * A status is read only when the count is not 0. A count that rose
 * for an event masked again before the read stays 1 (a phantom
 * event).
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define PORT      1
#define THROW_BIT 0x40000000u
#define POLLS     20000u
#define TAG       1

#define LE MFC_OUT_MBOX_AVAILABLE_EVENT
#define ME MFC_OUT_INTR_MBOX_AVAILABLE_EVENT
/* Privileged attention, bit 20 counted from the most significant end. */
#define A  0x00000800u

static volatile unsigned int out[16] __attribute__((aligned(16)));

static unsigned int intr_word(unsigned int port, unsigned int data0)
{
    return (port << 24) | (data0 & 0x00FFFFFFu);
}

static void quiet(void)
{
    spu_write_event_mask(0);
    spu_write_event_ack(0xFFFFFFFFu);
}

/* Read and acknowledge the status when the count is not 0. */
static unsigned int take_status(void)
{
    unsigned int status = 0;
    if (spu_stat_event_status() != 0) {
        status = spu_read_event_status();
        spu_write_event_ack(status);
    }
    return status;
}

/* Wait at most POLLS for the count to rise, then take the status. */
static unsigned int poll_status(void)
{
    unsigned int n;
    for (n = 0; n < POLLS && spu_stat_event_status() == 0; n++)
        ;
    return take_status();
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned int n;
    (void)arg0;
    (void)arg2;
    (void)arg3;

    /* 0: at start, A first */
    out[0] = spu_readch(SPU_RdEventMask);
    spu_write_event_mask(A);
    out[12] = spu_stat_event_status();
    out[13] = take_status();
    spu_write_event_mask(LE);
    out[1] = spu_stat_event_status();
    spu_write_event_mask(ME);
    out[2] = spu_stat_event_status();
    spu_write_event_mask(LE | ME);
    out[3] = take_status();

    /* 1: the outbound mailbox alone, then a send that drains both */
    quiet();
    spu_write_event_mask(LE | ME);
    spu_writech(SPU_WrOutMbox, 0x11111111u);
    out[4] = poll_status();
    spu_writech(SPU_WrOutIntrMbox, intr_word(PORT, 0x222222));
    out[5] = spu_readch(SPU_RdInMbox);
    out[6] = poll_status();
    out[7] = (spu_readchcnt(SPU_WrOutMbox) << 16) | spu_readchcnt(SPU_WrOutIntrMbox);

    /* 2: a throw with Me alone enabled, then Le enabled */
    quiet();
    spu_write_event_mask(ME);
    spu_writech(SPU_WrOutMbox, 0x33333333u);
    spu_writech(SPU_WrOutIntrMbox, THROW_BIT | intr_word(PORT, 0x444444));
    out[8] = poll_status();
    for (n = 0; n < POLLS && spu_readchcnt(SPU_WrOutMbox) == 0; n++)
        ;
    out[11] = spu_readchcnt(SPU_WrOutMbox);
    spu_write_event_mask(LE | ME);
    out[9] = spu_stat_event_status();
    out[10] = take_status();

    /* 3: A after the thread's mailbox traffic */
    quiet();
    spu_write_event_mask(A);
    out[14] = spu_stat_event_status();
    out[15] = take_status();
    quiet();

    mfc_put((void *)out, result_ea + 16, sizeof(out), TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
