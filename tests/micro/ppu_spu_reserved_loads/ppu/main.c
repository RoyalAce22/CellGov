/* PPU probe: what a PPU word load reads, or whether it ends the thread,
 * at an aligned and a misaligned address in two parts of the SPU address
 * range that hold nothing for this process: the first Raw SPU stride
 * when no Raw SPU exists, and the first SPU thread window slot when no
 * SPU thread group exists.
 *
 * The probe creates no SPU object and sends no store. Four cases run in
 * turn, each in its own joinable child thread, so a fault ends only that
 * thread:
 *
 *   case 1  lwz at 0xE0000000       (Raw SPU stride 0, aligned)
 *   case 2  lwz at 0xE0000002       (Raw SPU stride 0, misaligned)
 *   case 3  lwz at 0xF0000000       (SPU thread window slot 0, aligned)
 *   case 4  lwz at 0xF0000002       (SPU thread window slot 0, misaligned)
 *
 * Payload: big-endian u32 words.
 *    0  DONE ("SPRL") once the main thread finished, PROG in a
 *       progress record
 *    1  the step a progress record names (0 otherwise): 10 * case +
 *       1 the create, 2 the start, 3 the wait to reach the load, 4
 *       the wait to pass it
 *    2..9    case 1    10..17  case 2    18..25  case 3    26..33  case 4
 *       each: +0 child create rc, +1 start rc, +2 reached flag,
 *       +3 passed flag, +4 the loaded word (SKIP when it did not pass),
 *       +5 get-priority rc on the child after the wait, +6 join rc
 *       (SKIP when it did not pass), +7 the join's exit value (SKIP
 *       when it did not pass)
 *   34..35  zero
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

#define WORDS    36
#define DONE     0x5350524Cu /* "SPRL" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0xDEADD00Du

#define CASES      4
#define CASE_WORDS 8
#define CASE_BASE(n) (2 + CASE_WORDS * ((n) - 1))

#define THREAD_JOINABLE 1
#define CHILD_EXIT  0xE5
#define CHILD_PRIO  1000
#define STACK_SIZE  0x4000
#define BEFORE_TICKS 80000000ull  /* about one second of time base */
#define AFTER_TICKS  160000000ull /* about two seconds */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static const unsigned long CASE_EA[CASES] = {
    0xE0000000ul, 0xE0000002ul, 0xF0000000ul, 0xF0000002ul,
};

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int before;
static volatile unsigned int after;
static volatile unsigned int loaded;
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
    unsigned int value;
    (void)arg;
    before = 1;
    __sync_synchronize();
    __asm__ volatile("lwz %0, 0(%1)\n\t" : "=r"(value) : "b"(ea) : "memory");
    loaded = value;
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

static void run_case(unsigned int n)
{
    unsigned int w = CASE_BASE(n);

    before = after = 0;
    loaded = SKIP;
    access_ea = CASE_EA[n - 1];
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
        result[w + 4] = loaded;
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
    unsigned int i;

    for (i = 0; i < WORDS; i++)
        result[i] = i < 2 + CASES * CASE_WORDS ? SKIP : 0;
    progress(1);

    for (i = 1; i <= CASES; i++)
        run_case(i);

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
