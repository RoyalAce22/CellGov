/* SPU program: every slot of the SPU thread window, and the stop codes
 * that end or yield a thread group.
 *
 * arg0 is (mode << 8) | slot, arg1 the effective address of the result
 * block (ppu/main.c lists its layout).
 *
 * MODE_WINDOW, four threads of one group:
 *   - every slot fills PATTERN with 0x5A000000 | slot << 16 | word;
 *   - slots 1-3 each send the bit 1 << slot to slot 0's
 *     SPU_Sig_Notify_1, which the PPU set to OR mode, through slot 0's
 *     problem-state alias, then wait on their own register;
 *   - slot 0 waits until it has all three bits, gets PATTERN from every
 *     slot's local-store alias (its own included), puts the 64 bytes at
 *     +32 of the result block, then sends 0xB0 + slot to each other
 *     slot's SPU_Sig_Notify_1 through that slot's problem-state alias;
 *   - every slot puts { slot, word } at +96 + 16 * slot, where word is
 *     the bit mask (slot 0) or the release word (slots 1-3), and exits
 *     with sys_spu_thread_exit.
 *
 * MODE_GROUP_EXIT: one thread ends its group with
 * sys_spu_thread_group_exit(GROUP_EXIT_STATUS).
 *
 * MODE_YIELD: one thread yields its group, then puts YIELD_MARKER at
 * +160 and exits.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define WINDOW_BASE   0xF0000000u
#define WINDOW_STRIDE 0x00100000u
#define PROBLEM_STATE 0x00040000u
#define SIG_NOTIFY_1  0x0001400Cu

#define SLOTS 4
#define TAG   1

#define MODE_WINDOW     1
#define MODE_GROUP_EXIT 2
#define MODE_YIELD      3

#define GROUP_EXIT_STATUS 0x1234
#define YIELD_MARKER      0x59494C44u
#define RELEASE_BASE      0xB0u

static volatile unsigned int pattern[4] __attribute__((aligned(16)));
static volatile unsigned int gathered[SLOTS * 4] __attribute__((aligned(16)));
static volatile unsigned int record[4] __attribute__((aligned(16)));
/* A sndsig source word sits at the low four bits of the register's
 * effective address, 0xC. */
static volatile unsigned int signal_src[4] __attribute__((aligned(16)));

static void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

static void send_signal(unsigned int slot, unsigned int word)
{
    signal_src[3] = word;
    mfc_sndsig((void *)&signal_src[3],
               WINDOW_BASE + slot * WINDOW_STRIDE + PROBLEM_STATE + SIG_NOTIFY_1,
               TAG, 0, 0);
    wait_tag();
}

static void put_record(unsigned int result_ea, unsigned int at,
                       unsigned int w0, unsigned int w1)
{
    record[0] = w0;
    record[1] = w1;
    record[2] = 0;
    record[3] = 0;
    mfc_put((void *)record, result_ea + at, 16, TAG, 0, 0);
    wait_tag();
}

int main(unsigned long long arg0,
         unsigned long long arg_result_ea,
         unsigned long long arg2,
         unsigned long long arg3)
{
    unsigned int mode = (unsigned int)(arg0 >> 8);
    unsigned int slot = (unsigned int)(arg0 & 0xFF);
    unsigned int result_ea = (unsigned int)arg_result_ea;
    unsigned int i;

    (void)arg2;
    (void)arg3;

    if (mode == MODE_GROUP_EXIT) {
        spu_thread_group_exit(GROUP_EXIT_STATUS);
        return 0;
    }

    if (mode == MODE_YIELD) {
        spu_thread_group_yield();
        put_record(result_ea, 160, YIELD_MARKER, 0);
        spu_thread_exit(0);
        return 0;
    }

    for (i = 0; i < 4; i++)
        pattern[i] = 0x5A000000u | (slot << 16) | i;

    if (slot == 0) {
        unsigned int mask = 0;
        while (mask != 0xE)
            mask |= spu_readch(SPU_RdSigNotify1);

        for (i = 0; i < SLOTS; i++)
            mfc_get((void *)&gathered[i * 4],
                    WINDOW_BASE + i * WINDOW_STRIDE + (unsigned int)pattern,
                    16, TAG, 0, 0);
        wait_tag();
        mfc_put((void *)gathered, result_ea + 32, sizeof(gathered), TAG, 0, 0);
        wait_tag();

        for (i = 1; i < SLOTS; i++)
            send_signal(i, RELEASE_BASE + i);
        put_record(result_ea, 96, 0, mask);
    } else {
        unsigned int release;
        send_signal(0, 1u << slot);
        release = spu_readch(SPU_RdSigNotify1);
        put_record(result_ea, 96 + 16 * slot, slot, release);
    }

    spu_thread_exit(0);
    return 0;
}
