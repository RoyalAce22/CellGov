/* SPU program: an unconditional lock-line put takes another SPU's
 * reservation.
 *
 * Two threads of one group run this image. arg0 is the thread's slot in
 * the group, arg1 the effective address of a 48-byte result block, arg2
 * the effective address of a 128-byte lock line.
 *
 * Slot 0 (holder): getllar the line, signal slot 1, wait for slot 1's
 * signal, then putllc the line back. The putllc fails: slot 1's putlluc
 * took the reservation. It writes
 *   +0:  u32 status (0), u32 getllar status, u32 putllc status, u32 0
 *   +16: u8[16] the line's first bytes after the putllc
 *
 * Slot 1 (publisher): wait for slot 0's signal, putlluc a line of
 * PUBLISHED bytes, signal slot 0, and write
 *   +32: u32 putlluc status, u32 0, u32 0, u32 0
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define WINDOW_BASE   0xF0000000u
#define WINDOW_STRIDE 0x00100000u
#define PROBLEM_STATE 0x00040000u
#define SIG_NOTIFY_1  0x0001400Cu

#define TAG       1
#define PUBLISHED 0x77

static volatile unsigned char line[128] __attribute__((aligned(128)));
static volatile unsigned int signal_src[4] __attribute__((aligned(16)));
static volatile unsigned int result[8] __attribute__((aligned(16)));

/* sndsig the word at signal_src[3] to `slot`'s SPU_Sig_Notify_1. */
static void signal_slot(unsigned int slot)
{
    unsigned int reg = WINDOW_BASE + slot * WINDOW_STRIDE + PROBLEM_STATE + SIG_NOTIFY_1;
    signal_src[3] = 1;
    mfc_sndsig((void *)&signal_src[3], reg, TAG, 0, 0);
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long slot,
         unsigned long long arg_result_ea,
         unsigned long long arg_line_ea,
         unsigned long long arg4)
{
    unsigned int result_ea = (unsigned int)arg_result_ea;
    /* The PPU passes 32-bit addresses; only the low word of each
     * argument carries one. */
    unsigned long long line_ea = (unsigned int)arg_line_ea;
    (void)arg4;

    if (slot == 0) {
        unsigned int got, put;

        mfc_getllar((void *)line, line_ea, 0, 0);
        got = mfc_read_atomic_status();
        signal_slot(1);
        spu_readch(SPU_RdSigNotify1);

        mfc_putllc((void *)line, line_ea, 0, 0);
        put = mfc_read_atomic_status();

        mfc_get((void *)line, line_ea, 128, TAG, 0, 0);
        mfc_write_tag_mask(1 << TAG);
        mfc_read_tag_status_all();

        result[0] = 0;
        result[1] = got;
        result[2] = put;
        result[3] = 0;
        result[4] = ((volatile unsigned int *)line)[0];
        result[5] = ((volatile unsigned int *)line)[1];
        result[6] = ((volatile unsigned int *)line)[2];
        result[7] = ((volatile unsigned int *)line)[3];
        mfc_put((void *)result, result_ea, 32, TAG, 0, 0);
    } else {
        unsigned int published;
        int i;

        spu_readch(SPU_RdSigNotify1);
        for (i = 0; i < 128; i++)
            line[i] = PUBLISHED;
        mfc_putlluc((void *)line, line_ea, 0, 0);
        published = mfc_read_atomic_status();
        signal_slot(0);

        result[0] = published;
        result[1] = 0;
        result[2] = 0;
        result[3] = 0;
        mfc_put((void *)result, result_ea + 32, 16, TAG, 0, 0);
    }
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();

    spu_thread_exit(0);
    return 0;
}
