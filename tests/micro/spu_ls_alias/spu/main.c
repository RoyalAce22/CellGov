/* SPU program: SPU-to-SPU transfer through the SPU thread window.
 *
 * Two threads of one group run this image. arg0 is the thread's slot in
 * the group, arg1 the effective address of a 32-byte result block.
 *
 * Slot 0 (sender): fills a 16-byte buffer in its own local store, then
 * sends SIGNAL_WORD to slot 1's SPU_Sig_Notify_1 with sndsig through
 * slot 1's problem-state alias in the window.
 *
 * Slot 1 (receiver): reads SPU_RdSigNotify1, which parks until the
 * signal arrives, then gets the sender's buffer through slot 0's local
 * store alias and writes the result block:
 *   +0:  u32 status (0), u32 signal word, u32 slot, u32 0
 *   +16: u8[16] the bytes the get read from the sender's local store
 *
 * The receiver gets rather than the sender puts: the reference runner
 * refuses a put into another thread's alias, and serves a get and a
 * sndsig.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define WINDOW_BASE   0xF0000000u
#define WINDOW_STRIDE 0x00100000u
#define PROBLEM_STATE 0x00040000u
#define SIG_NOTIFY_1  0x0001400Cu

#define LS_INBOX  0x3000
#define LS_RESULT 0x3100
#define TAG       1

#define SIGNAL_WORD 0xC0DE0001u

/* The sndsig source: the word's local-store address must share its low
 * four bits with the register's effective address, 0xC. */
static volatile unsigned int signal_src[4] __attribute__((aligned(16)));
static volatile unsigned char payload[16] __attribute__((aligned(16)));

int main(unsigned long long slot,
         unsigned long long arg_result_ea,
         unsigned long long arg3,
         unsigned long long arg4)
{
    (void)arg3;
    (void)arg4;

    if (slot == 0) {
        unsigned int peer = WINDOW_BASE + 1 * WINDOW_STRIDE;
        int i;
        for (i = 0; i < 16; i++)
            payload[i] = (unsigned char)(0xA0 + i);
        signal_src[3] = SIGNAL_WORD;

        mfc_sndsig((void *)&signal_src[3], peer + PROBLEM_STATE + SIG_NOTIFY_1, TAG, 0, 0);
        mfc_write_tag_mask(1 << TAG);
        mfc_read_tag_status_all();
    } else {
        unsigned int sender = WINDOW_BASE + 0 * WINDOW_STRIDE;
        unsigned int result_ea = (unsigned int)arg_result_ea;
        volatile unsigned int *result = (volatile unsigned int *)LS_RESULT;
        volatile unsigned int *inbox = (volatile unsigned int *)LS_INBOX;
        unsigned int word = spu_readch(SPU_RdSigNotify1);

        mfc_get((void *)LS_INBOX, sender + (unsigned int)payload, 16, TAG, 0, 0);
        mfc_write_tag_mask(1 << TAG);
        mfc_read_tag_status_all();

        result[0] = 0;
        result[1] = word;
        result[2] = (unsigned int)slot;
        result[3] = 0;
        result[4] = inbox[0];
        result[5] = inbox[1];
        result[6] = inbox[2];
        result[7] = inbox[3];

        mfc_put((void *)LS_RESULT, result_ea, 32, TAG, 0, 0);
        mfc_write_tag_mask(1 << TAG);
        mfc_read_tag_status_all();
    }

    spu_thread_exit(0);
    return 0;
}
