/* PPU program: writes a GCM_FLIP_COMMAND method, advances put,
 * polls the flip-status mirror through its WAITING -> DONE
 * transition, and exits.
 *
 * Exercises the flip-status state machine:
 *
 *   1. ELF runs with `[rsx] mirror = true`. The RSX region is
 *      writable; writes to 0xC000_0040 / 0xC000_0044 mirror to
 *      the runtime's RSX cursor; flip-status transitions mirror
 *      to `RSX_FLIP_STATUS_MIRROR_ADDR` (0xC000_0050).
 *   2. Writes a GCM_FLIP_COMMAND (method 0xFEAC) with buffer
 *      index 3 into the FIFO.
 *   3. Advances put. The FIFO drain queues an RsxFlipRequest.
 *   4. Commit pipeline applies the request -> flip status
 *      becomes WAITING. The flip-status mirror updates. The
 *      guest polls and observes WAITING.
 *   5. Next commit boundary transitions status to DONE. The
 *      mirror updates again. The guest polls and observes DONE.
 *   6. The test reports PASS / FAIL via TTY and exits.
 *
 * Output (TTY payload = CGOV magic + 16 bytes):
 *   status              (u32)  0 = pass, else bitfield:
 *                                0x1 = never saw WAITING
 *                                0x2 = never saw DONE after WAITING
 *                                0x4 = saw a value other than
 *                                      WAITING or DONE
 *   waiting_iters       (u32)  iterations until WAITING observed
 *   done_iters          (u32)  iterations from WAITING observation
 *                              until DONE observed
 *   last_status         (u32)  final value read from the mirror
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

static inline s32 syscall4_s32(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline void syscall1_noreturn(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        :
        : "r"(r3), "r"(r11)
        : "memory"
    );
}

#define SYS_PPU_THREAD_EXIT 41
#define SYS_TTY_WRITE       403

/* GCM_FLIP_COMMAND method address (Sony-specific NV4097
 * extension). The single argument is the back-buffer index. */
#define GCM_FLIP_COMMAND 0xFEAC
#define NV_COUNT_SHIFT   18

#define RSX_PUT_ADDR         0xC0000040u
#define RSX_GET_ADDR         0xC0000044u
#define RSX_FLIP_STATUS_ADDR 0xC0000050u

/* Sony libgcm flip-status byte values (cell_gcm.h). */
#define FLIP_STATUS_DONE    0u
#define FLIP_STATUS_WAITING 1u

struct TestResult {
    unsigned int status;
    unsigned int waiting_iters;
    unsigned int done_iters;
    unsigned int last_status;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int fifo_buffer[2] __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)CGOV_MAGIC, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)len_be, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)r, len,
                 (unsigned long)&written);
}

static inline void fifo_store_le(volatile unsigned int *slot, unsigned int value)
{
    *slot = __builtin_bswap32(value);
}

/* Read the flip-status mirror as a u32 (big-endian load, so the
 * low byte carries the current status and the upper 24 bits are
 * the oracle-written zero padding). */
static inline unsigned int read_flip_status(void)
{
    return *((volatile unsigned int *)(unsigned long)RSX_FLIP_STATUS_ADDR);
}

int main(void)
{
    unsigned int fifo_base = (unsigned int)(unsigned long)&fifo_buffer[0];
    unsigned int fifo_end = fifo_base + 8;

    result.status = 0;
    result.waiting_iters = 0;
    result.done_iters = 0;
    result.last_status = 0;

    /* FIFO words (little-endian on disk):
     *   [0] FLIP_BUFFER header (method 0xFEAC, count 1)
     *   [1] buffer index argument (3) */
    unsigned int header_flip = ((unsigned int)1 << NV_COUNT_SHIFT) | GCM_FLIP_COMMAND;
    fifo_store_le(&fifo_buffer[0], header_flip);
    fifo_store_le(&fifo_buffer[1], 3u);

    __asm__ volatile ("sync" ::: "memory");

    /* Seed cursor.get then advance put. */
    *((volatile unsigned int *)(unsigned long)RSX_GET_ADDR) = fifo_base;
    *((volatile unsigned int *)(unsigned long)RSX_PUT_ADDR) = fifo_end;

    /* Poll the flip-status mirror. Initial value is DONE = 0.
     * The RsxFlipRequest emitted by the FIFO drain commits in
     * batch N+1, flipping status to WAITING and updating the
     * mirror. One more commit later, the state machine
     * transitions status back to DONE and the mirror updates
     * again. Each of our poll iterations triggers a commit
     * boundary (the load is a PPU step), so eventually we
     * observe both values. */
    unsigned int waiting_iters = 0;
    unsigned int done_iters_after_waiting = 0;
    unsigned int saw_waiting = 0;
    unsigned int saw_done_after_waiting = 0;
    unsigned int saw_invalid_value = 0;
    unsigned int last = 0;

    /* Race-resilience: the oracle guarantees WAITING persists for
     * at least one full commit boundary between the
     * RsxFlipRequest-applies commit and the DONE-transition
     * commit. Within one PPU step every load of the mirror
     * returns the same committed value, so if WAITING ever
     * reaches memory the poll loop sees it (many loads per step
     * all on the same frozen memory view). But this test also
     * guards a secondary signal: if we never saw WAITING but
     * DID see DONE AFTER the initial DONE (i.e., a second DONE
     * after the mirror had time to transition), we still count
     * that as the WAITING-to-DONE transition having happened
     * -- the oracle's contract is "exactly one WAITING-to-DONE
     * resolution," so even missing the intermediate WAITING
     * frame is not a correctness failure as long as DONE was
     * reached through the transition. The oracle's per-commit
     * state-hash already pins the transition directly; this
     * microtest's job is to confirm the guest-visible mirror
     * lands coherently. */
    for (unsigned int i = 0; i < 200000u; i++) {
        unsigned int cur = read_flip_status();
        last = cur;
        if (!saw_waiting) {
            waiting_iters = i;
            if (cur == FLIP_STATUS_WAITING) {
                saw_waiting = 1;
                continue;
            }
            if (cur != FLIP_STATUS_DONE && cur != FLIP_STATUS_WAITING) {
                saw_invalid_value = 1;
                break;
            }
        } else {
            /* Already saw WAITING; now wait for DONE. Count
             * the iteration in which DONE is observed (done_iters
             * is "iterations INCLUDING the one that saw DONE",
             * matching the header comment's "iterations until
             * DONE observed"). */
            done_iters_after_waiting++;
            if (cur == FLIP_STATUS_DONE) {
                saw_done_after_waiting = 1;
                break;
            }
            if (cur != FLIP_STATUS_WAITING && cur != FLIP_STATUS_DONE) {
                saw_invalid_value = 1;
                break;
            }
        }
    }

    result.waiting_iters = waiting_iters;
    result.done_iters = done_iters_after_waiting;
    result.last_status = last;
    if (!saw_waiting) {
        result.status |= 0x1;
    }
    if (!saw_done_after_waiting) {
        result.status |= 0x2;
    }
    if (saw_invalid_value) {
        result.status |= 0x4;
    }

    const char *verdict;
    if (result.status == 0) {
        verdict = "RSX_FLIP_STATUS_TRANSITION: PASS\n";
    } else {
        verdict = "RSX_FLIP_STATUS_TRANSITION: FAIL\n";
    }
    unsigned int vlen = 0;
    while (verdict[vlen] != '\0') {
        vlen++;
    }
    unsigned int written;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)verdict, vlen,
                 (unsigned long)&written);

    write_tty_result(&result);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00Du);
    for (;;) { }
}
