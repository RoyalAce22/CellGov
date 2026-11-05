/* PPU probe: mtmsrd r3 (a Book III instruction) in a child thread.
 *
 * The instruction runs in a joinable child thread, so the main thread
 * can record what the process sees. The child sets `before`, runs the
 * instruction, stores its destination register and sets `after`, then
 * exits with CHILD_EXIT. The main thread writes a progress record before
 * each step, so a run that ends at a step leaves that record. It joins
 * the child only once `after` is set: a join on a thread that never
 * exits would hold the title open.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("EXCP") once the main thread finished, PROGRESS ("PROG")
 *      in a progress record
 *   1  the step a progress record names (0 otherwise): 1 the create,
 *      2 the start, 3 the wait for `before`, 4 the wait for `after`
 *   2  the create's return code
 *   3  the start's return code
 *   4  `before` once the main thread stopped waiting for it
 *   5  `after` once the main thread stopped waiting for it
 *   6..7  the destination register after the instruction, high and low
 *      word (preset 0xDEADBEEF_DEADBEEF), 0 when `after` is 0
 *   8  syscall 48 (get priority) on the child after the wait: 0 while
 *      the thread exists
 *   9  the join's return code, SKIP when `after` is 0
 *   10  the exit value the join returned, SKIP when `after` is 0
 *   11  the low 32 bits of the access's effective address (0 for a
 *      case with no access)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define KIND_MTMSRD 1
#define ACCESS 0
#define ACCESS_OFFSET 0

#define WORDS 16
#define DONE     0x45584350u /* "EXCP" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0x5C1F5C1Fu

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_GET_PRIORITY 48

#define THREAD_JOINABLE 1
#define CHILD_EXIT  0xE5
#define CHILD_PRIO  1000
#define STACK_SIZE  0x4000
#define BEFORE_TICKS 80000000ull  /* about one second of time base */
#define AFTER_TICKS  160000000ull /* about two seconds */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char window[64] __attribute__((aligned(128)));
static volatile unsigned int before;
static volatile unsigned int after;
static volatile unsigned long long dest;
static volatile unsigned long access_ea;

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
    unsigned long long value = 0xDEADBEEFDEADBEEFull;
    unsigned long ea = access_ea;
    (void)arg;
    (void)ea;
    before = 1;
    __sync_synchronize();
#if KIND_ILLEGAL
    __asm__ volatile(".long 0x04000000\n\t" ::: "memory");
#elif KIND_MTMSRD
    __asm__ volatile("li 3, 0\n\t.long 0x7C600164\n\t" ::: "r3", "memory");
#elif KIND_TW
    __asm__ volatile(".long 0x7FE00008\n\t" ::: "memory");
#elif KIND_TWI_FALSE
    __asm__ volatile("li 3, 0\n\ttwi 4, 3, 1\n\t" ::: "r3", "memory");
#elif KIND_LWARX
    __asm__ volatile("lwarx %0, 0, %1\n\t" : "+r"(value) : "b"(ea) : "memory");
#elif KIND_LDARX
    __asm__ volatile("ldarx %0, 0, %1\n\t" : "+r"(value) : "b"(ea) : "memory");
#elif KIND_LWZ
    __asm__ volatile("lwz %0, 0(%1)\n\t" : "+r"(value) : "b"(ea) : "memory");
#endif
    dest = value;
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

int main(void)
{
    unsigned int i;

    memset((void *)result, 0, sizeof(result));
    for (i = 0; i < sizeof(window); i++)
        window[i] = (unsigned char)(0x10 + i);
    before = after = 0;
    dest = 0;
#if ACCESS
    access_ea = (unsigned long)window + ACCESS_OFFSET;
#else
    access_ea = 0;
#endif
    result[11] = (unsigned int)access_ea;

    progress(1);
    result[2] = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, (u64)(unsigned long)&tid,
                                           make_param(&param, (const void *)&child), 0, 0,
                                           CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
    if (result[2] != 0) {
        write_frame();
        return 0;
    }
    progress(2);
    result[3] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
    if (result[3] != 0) {
        write_frame();
        return 0;
    }
    progress(3);
    result[4] = wait_flag(&before, BEFORE_TICKS);
    progress(4);
    result[5] = wait_flag(&after, AFTER_TICKS);
    __sync_synchronize();
    if (result[5]) {
        result[6] = (unsigned int)(dest >> 32);
        result[7] = (unsigned int)dest;
    }
    result[8] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, tid,
                                           (u64)(unsigned long)&prio);
    if (result[5]) {
        exit_value = 0xFFFFFFFFFFFFFFFFull;
        result[9] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid,
                                               (u64)(unsigned long)&exit_value);
        result[10] = (unsigned int)exit_value;
    } else {
        result[9] = SKIP;
        result[10] = SKIP;
    }

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
