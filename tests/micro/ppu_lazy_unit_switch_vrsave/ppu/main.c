/* PPU program: whether the data stream instructions and the VRSAVE
 * moves run with the vector unit off, on a thread the kernel resumes
 * with MSR[VXU] clear (the ppu_lazy_unit_switch capture shows that it
 * does).
 *
 * Each round makes the kernel switch the thread out with
 * sys_timer_usleep (141) of SLEEP_US, then times, in this order: one
 * candidate instruction, the same candidate again (warm), a vector add
 * (vaddubm) and the vector add again (warm). A candidate that runs with
 * the unit off costs about what the integer candidate costs, and the
 * vector add after it still costs an unavailable interrupt. A candidate
 * that takes the interrupt costs it itself, and the vector add after it
 * runs warm.
 *
 * Candidates, ROUNDS rounds each, in this order:
 *   0 add r9,r9,r9 (the control)   1 mfspr r9,256 (mfvrsave)
 *   2 mtspr 256,r9, r9 holding VRSAVE as mfvrsave read it
 *   3 dss 0                         4 dst r9,r10,0 on `line`
 * A dst round stops its stream with dss 0 after its vector adds.
 *
 * Calls: 141 only.
 *
 * Result layout (big-endian u32 words, 672 bytes):
 *   w0: failed step (0 = none), w1: its return code
 *   w2: candidates (CANDS), w3: rounds per candidate (ROUNDS)
 *   w4..w7: zero
 *   w8 + 4*(ROUNDS*c + r) + k: candidate c, round r, time-base ticks of
 *       k = 0 candidate first, 1 candidate warm, 2 vaddubm first after
 *       it, 3 vaddubm warm
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CANDS        5
#define ROUNDS       8
#define BASE         8
#define WORDS        (BASE + 4 * CANDS * ROUNDS)
#define RESULT_BYTES (WORDS * 4)

#define SYS_TIMER_USLEEP 141
#define SLEEP_US         100

/* dst r9,r10,0: one 16-byte block, count 1, stride 0. */
#define DST_CONTROL 0x01010000ull

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[WORDS] __attribute__((aligned(128)));
static volatile unsigned char line[128] __attribute__((aligned(128)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result_buf[0] = step;
    result_buf[1] = (unsigned int)rc;
    write_frame(result_buf, RESULT_BYTES);
    return (int)step;
}

/* Ticks across one instruction. r9 and r10 are loaded before the first
 * mftb. dss 0 is 0x7C00066C, dst r9,r10,0 is 0x7C0952AC and vaddubm
 * v1,v2,v3 is 0x10221800, spelled as words. */
static unsigned int time_cand(unsigned int c)
{
    unsigned long long a, b;
    unsigned long long ea = (unsigned long long)(unsigned long)line;
    switch (c) {
    case 0:
        __asm__ volatile ("li 9,1\n\tmftb %0\n\tadd 9,9,9\n\tmftb %1"
                          : "=&r"(a), "=&r"(b) :: "r9", "memory");
        break;
    case 1:
        __asm__ volatile ("mftb %0\n\tmfspr 9,256\n\tmftb %1"
                          : "=&r"(a), "=&r"(b) :: "r9", "memory");
        break;
    case 2:
        /* mfspr before the first mftb: it is case 1's candidate. */
        __asm__ volatile ("mfspr 9,256\n\tmftb %0\n\tmtspr 256,9\n\tmftb %1"
                          : "=&r"(a), "=&r"(b) :: "r9", "memory");
        break;
    case 3:
        __asm__ volatile ("mftb %0\n\t.long 0x7C00066C\n\tmftb %1"
                          : "=&r"(a), "=&r"(b) :: "memory");
        break;
    default:
        __asm__ volatile ("mr 9,%2\n\tmr 10,%3\n\tmftb %0\n\t.long 0x7C0952AC\n\tmftb %1"
                          : "=&r"(a), "=&r"(b)
                          : "r"(ea), "r"(DST_CONTROL)
                          : "r9", "r10", "memory");
        break;
    }
    return (unsigned int)(b - a);
}

static unsigned int time_vec(void)
{
    unsigned long long a, b;
    __asm__ volatile ("mftb %0\n\t.long 0x10221800\n\tmftb %1"
                      : "=&r"(a), "=&r"(b) :: "memory", "v1");
    return (unsigned int)(b - a);
}

static void stop_streams(void)
{
    __asm__ volatile (".long 0x7C00066C" ::: "memory");
}

int main(void)
{
    unsigned int rc;

    memset((void *)result_buf, 0, sizeof(result_buf));
    result_buf[2] = CANDS;
    result_buf[3] = ROUNDS;

    /* Use the vector unit once, so no first use in the process is timed. */
    (void)time_vec();

    for (unsigned int c = 0; c < CANDS; c++) {
        for (unsigned int r = 0; r < ROUNDS; r++) {
            unsigned int d[4];
            rc = (unsigned int)syscall1_s32(SYS_TIMER_USLEEP, SLEEP_US);
            if (rc != 0) return fail(1 + c, (int)rc);
            d[0] = time_cand(c);
            d[1] = time_cand(c);
            d[2] = time_vec();
            d[3] = time_vec();
            if (c == 4)
                stop_streams();
            for (unsigned int k = 0; k < 4; k++)
                result_buf[BASE + 4 * (ROUNDS * c + r) + k] = d[k];
        }
    }

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
