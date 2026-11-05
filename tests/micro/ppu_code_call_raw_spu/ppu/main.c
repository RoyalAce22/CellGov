/* PPU probe: what a PPU branch into a Raw SPU's local-store window does.
 *
 * The probe creates one Raw SPU a (169 with one Raw SPU, then 160); it
 * is not started. The main thread stores two PPU instructions into a's
 * local store words 0 and 1 by aligned word stores:
 *
 *   li r3,0x77     0x38600077
 *   blr            0x4E800020
 *
 * Two cases then run in turn, each in its own joinable child thread, so
 * a fault ends only that thread:
 *
 *   case 1  bctrl to ret77, a function in the probe's own code that
 *           does the same two instructions (the control)
 *   case 2  bctrl to window(a) + 0
 *
 * window(id) is 0xE0000000 + id * 0x100000.
 *
 * Payload: big-endian u32 words.
 *    0  DONE ("RAWX") once the main thread finished, PROG in a
 *       progress record
 *    1  the step a progress record names (0 otherwise): 10 * case +
 *       1 the create, 2 the start, 3 the wait to reach the call, 4 the
 *       wait to pass it
 *    2  169 rc   3  160 rc   4  a   5  161 a
 *    6  a's local store word 0 read back   7  word 1 read back
 *    8..15   case 1     16..23  case 2
 *       each: +0 child create rc, +1 start rc, +2 reached flag,
 *       +3 passed flag, +4 r3 after the call (SKIP when it did not
 *       pass), +5 get-priority rc on the child after the wait, +6 join
 *       rc (SKIP when it did not pass), +7 the join's exit value (SKIP
 *       when it did not pass)
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

#define WORDS    24
#define DONE     0x52415758u /* "RAWX" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0xDEADD00Du

#define CASE_WORDS 8
#define CASE_BASE(n) (8 + CASE_WORDS * ((n) - 1))

#define LI_R3_77 0x38600077u
#define BLR      0x4E800020u

#define THREAD_JOINABLE 1
#define CHILD_EXIT  0xE5
#define CHILD_PRIO  1000
#define STACK_SIZE  0x4000
#define BEFORE_TICKS 80000000ull  /* about one second of time base */
#define AFTER_TICKS  160000000ull /* about two seconds */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int before;
static volatile unsigned int after;
static volatile unsigned int returned;
static volatile unsigned long call_ea;
static volatile unsigned int g_id;

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

/* The control's target: the same two instructions in the probe's code. */
__asm__(
    ".text\n"
    ".p2align 2\n"
    "ret77_code:\n"
    "    li 3,0x77\n"
    "    blr\n");

static unsigned long ret77_ea(void)
{
    unsigned long ea;
    __asm__ volatile ("lis %0,ret77_code@highest\n\t"
                      "ori %0,%0,ret77_code@higher\n\t"
                      "rldicr %0,%0,32,31\n\t"
                      "oris %0,%0,ret77_code@h\n\t"
                      "ori %0,%0,ret77_code@l" : "=r"(ea));
    return ea;
}

static void child(u64 arg)
{
    unsigned long ea = call_ea;
    unsigned long r;
    (void)arg;
    before = 1;
    __sync_synchronize();
    __asm__ volatile("mtctr %1\n\t"
                     "bctrl\n\t"
                     "mr %0,3\n\t"
                     : "=r"(r)
                     : "r"(ea)
                     : "r3", "ctr", "lr", "memory");
    returned = (unsigned int)r;
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

static void run_case(unsigned int n, unsigned long ea)
{
    unsigned int w = CASE_BASE(n);

    before = after = 0;
    returned = SKIP;
    call_ea = ea;
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
        if (result[w + 3])
            result[w + 4] = returned;
        result[w + 5] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio));
        if (result[w + 3]) {
            exit_value = 0xFFFFFFFFFFFFFFFFull;
            result[w + 6] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, EA(&exit_value));
            result[w + 7] = (unsigned int)exit_value;
        }
    }
}

int main(void)
{
    unsigned int i, a, a_ok;

    for (i = 0; i < WORDS; i++)
        result[i] = SKIP;
    progress(1);

    result[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 1);
    g_id = 0xEEEEEEEE;
    result[3] = (unsigned int)syscall2_s32(SYS_RAW_SPU_CREATE, EA(&g_id), 0);
    result[4] = g_id;
    a = g_id;
    a_ok = result[3] == 0;

    run_case(1, ret77_ea());
    if (a_ok) {
        LS_WORD(a, 0) = LI_R3_77;
        LS_WORD(a, 4) = BLR;
        __sync_synchronize();
        result[6] = LS_WORD(a, 0);
        result[7] = LS_WORD(a, 4);
        run_case(2, (unsigned long)RAW_BASE(a));
        result[5] = (unsigned int)syscall1_s32(SYS_RAW_SPU_DESTROY, a);
    }

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
