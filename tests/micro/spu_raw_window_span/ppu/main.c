/* PPU probe: what a PPU store does when it is misaligned inside a Raw
 * SPU's window, and when it runs from one Raw SPU's window into the
 * next one's.
 *
 * The probe creates two Raw SPUs a and b (169 with two Raw SPUs, then
 * 160 twice); neither is started. The main thread fills b's local
 * store words 0..3 with 0xA0A1A2A3, 0xB0B1B2B3, 0xC0C1C2C3, 0xD0D1D2D3
 * by aligned word stores. Three cases then run in turn, each in its own
 * joinable child thread, so a fault ends only that thread:
 *
 *   case 1  stw 0x5A5A5A5A at window(b) + 12     (aligned; the control)
 *   case 2  std 0x1122334455667788 at window(b) + 4   (misaligned, in
 *           b's local store)
 *   case 3  std 0x1122334455667788 at window(b) - 4   (four bytes in
 *           the end of a's window, four in b's local store word 0)
 *
 * window(id) is 0xE0000000 + id * 0x100000. Case 3 runs only when b is
 * a + 1, so window(b) - 4 lies in a's window.
 *
 * Payload: big-endian u32 words.
 *    0  DONE ("RAWS") once the main thread finished, PROG in a
 *       progress record
 *    1  the step a progress record names (0 otherwise): 10 * case +
 *       1 the create, 2 the start, 3 the wait to reach the store, 4
 *       the wait to pass it
 *    2  169 rc   3  160 a rc   4  a   5  160 b rc   6  b
 *    7  161 b    8  161 a
 *    9..19   case 1     20..30  case 2     31..41  case 3
 *       each: +0 child create rc, +1 start rc, +2 reached flag,
 *       +3 passed flag, +4 get-priority rc on the child after the
 *       wait, +5 join rc (SKIP when it did not pass), +6 the join's
 *       exit value (SKIP when it did not pass), +7..+10 b's local
 *       store words 0..3 after the case
 *   42..47  zero
 *
 * SKIP is 0xDEADD00D. A case that does not run leaves its words SKIP.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_GET_PRIORITY 48
#define SYS_RAW_SPU_CREATE          160
#define SYS_RAW_SPU_DESTROY         161
#define SYS_SPU_INITIALIZE          169

#define RAW_BASE(id) (0xE0000000ull + (u64)(id) * 0x100000ull)
#define LS_WORD(id, off) \
    (*(volatile unsigned int *)(unsigned long)(RAW_BASE(id) + (off)))

#define WORDS    48
#define DONE     0x52415753u /* "RAWS" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0xDEADD00Du

#define CASE_WORDS 11
#define CASE_BASE(n) (9 + CASE_WORDS * ((n) - 1))

#define THREAD_JOINABLE 1
#define CHILD_EXIT  0xE5
#define CHILD_PRIO  1000
#define STACK_SIZE  0x4000
#define BEFORE_TICKS 80000000ull  /* about one second of time base */
#define AFTER_TICKS  160000000ull /* about two seconds */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int g_id;
static volatile unsigned int before;
static volatile unsigned int after;
static volatile unsigned int access_case;
static volatile unsigned long access_ea;

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static void write_file(void)
{
    CGOV_OUT_FILE_WRITE((const void *)result, sizeof(result));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    write_file();
}

static void progress(unsigned int step)
{
    result[0] = PROGRESS;
    result[1] = step;
    write_file();
}

/* The toolchain's &fn is a 24-byte ELFv1 descriptor; syscall 52's
 * thread-init block names an 8-byte { u32 code; u32 toc } one. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned int self_tls(void)
{
    unsigned long r13;
    __asm__ volatile ("mr %0, 13" : "=r"(r13));
    return (unsigned int)r13;
}

static u64 make_param(struct thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = self_tls();
    return (u64)(unsigned long)p;
}

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static unsigned int wait_flag(volatile unsigned int *flag, unsigned long long ticks)
{
    unsigned long long t0 = timebase();
    while (timebase() - t0 < ticks) {
        if (*flag)
            return 1;
    }
    return *flag ? 1 : 0;
}

static void child(u64 arg)
{
    unsigned long ea = access_ea;
    unsigned int word = 0x5A5A5A5Au;
    unsigned long long dword = 0x1122334455667788ull;
    (void)arg;
    before = 1;
    __sync_synchronize();
    if (access_case == 1)
        __asm__ volatile("stw %0, 0(%1)\n\t" : : "r"(word), "b"(ea) : "memory");
    else
        __asm__ volatile("std %0, 0(%1)\n\t" : : "r"(dword), "b"(ea) : "memory");
    __sync_synchronize();
    after = 1;
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, CHILD_EXIT);
    for (;;) { }
}

static struct thread_param param __attribute__((aligned(8)));
static unsigned long long tid;
static unsigned long long exit_value;
static int prio;

static void run_case(unsigned int n, unsigned long ea, unsigned int b)
{
    unsigned int w = CASE_BASE(n), i;

    before = after = 0;
    access_case = n;
    access_ea = ea;
    __sync_synchronize();
    progress(10 * n + 1);
    result[w + 0] = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, EA(&tid),
                                               make_param(&param, (const void *)&child), 0, 0,
                                               CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
    if (result[w + 0] == 0) {
        progress(10 * n + 2);
        result[w + 1] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
    }
    if (result[w + 0] == 0 && result[w + 1] == 0) {
        progress(10 * n + 3);
        result[w + 2] = wait_flag(&before, BEFORE_TICKS);
        progress(10 * n + 4);
        result[w + 3] = wait_flag(&after, AFTER_TICKS);
        __sync_synchronize();
        result[w + 4] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio));
        if (result[w + 3]) {
            exit_value = 0xFFFFFFFFFFFFFFFFull;
            result[w + 5] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, EA(&exit_value));
            result[w + 6] = (unsigned int)exit_value;
        }
    }
    for (i = 0; i < 4; i++)
        result[w + 7 + i] = LS_WORD(b, 4 * i);
}

int main(void)
{
    unsigned int i, a, b, a_ok, b_ok;

    for (i = 0; i < WORDS; i++)
        result[i] = i < 42 ? SKIP : 0;
    progress(1);

    result[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 2);
    g_id = 0xEEEEEEEE;
    result[3] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, EA(&g_id), 0);
    result[4] = g_id;
    a = g_id;
    a_ok = result[3] == 0;
    b_ok = 0;
    b = 0;
    if (a_ok) {
        g_id = 0xEEEEEEEE;
        result[5] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, EA(&g_id), 0);
        result[6] = g_id;
        b = g_id;
        b_ok = result[5] == 0;
    }

    if (a_ok && b_ok) {
        LS_WORD(b, 0)  = 0xA0A1A2A3u;
        LS_WORD(b, 4)  = 0xB0B1B2B3u;
        LS_WORD(b, 8)  = 0xC0C1C2C3u;
        LS_WORD(b, 12) = 0xD0D1D2D3u;
        run_case(1, (unsigned long)(RAW_BASE(b) + 12), b);
        run_case(2, (unsigned long)(RAW_BASE(b) + 4), b);
        if (b == a + 1)
            run_case(3, (unsigned long)(RAW_BASE(b) - 4), b);
    }

    if (b_ok)
        result[7] = (unsigned int)syscall1_s32(SYS_RAW_SPU_DESTROY, b);
    if (a_ok)
        result[8] = (unsigned int)syscall1_s32(SYS_RAW_SPU_DESTROY, a);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
