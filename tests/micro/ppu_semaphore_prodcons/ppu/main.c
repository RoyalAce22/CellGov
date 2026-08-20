/* PPU program: producer-consumer bounded buffer protected by two
 * counting semaphores. Producer thread posts N messages through a
 * circular buffer; consumer thread reads them and sums. Proves
 * sys_semaphore_wait / _post semantics including the wake-or-
 * increment rule.
 *
 * Structural microtest for the semaphore primitive. It proves:
 *
 *   1. sys_semaphore_create with (initial, max) bounds is
 *      honored and the id is written to id_ptr.
 *   2. sys_semaphore_wait on zero count blocks the caller.
 *   3. sys_semaphore_post with a parked waiter wakes that
 *      waiter and does NOT increment count (FIFO-fair).
 *   4. sys_semaphore_post with no waiters increments count.
 *   5. A bounded-buffer exchange with N messages passes every
 *      message through without loss, regardless of how the
 *      scheduler interleaves producer and consumer.
 *
 * Bounded-buffer protocol with BUF_SIZE slots:
 *   - space_sem: counts free slots (initial = BUF_SIZE, max =
 *                BUF_SIZE)
 *   - data_sem:  counts filled slots (initial = 0, max = BUF_SIZE)
 *   Producer: for i in 0..N:
 *               wait(space_sem); buf[i % BUF_SIZE] = msg(i);
 *               post(data_sem);
 *   Consumer: for i in 0..N:
 *               wait(data_sem); sum += buf[i % BUF_SIZE];
 *               post(space_sem);
 *
 * Output layout (TTY: CGOV magic + 16 bytes):
 *   status         (u32) 0 = pass, bitfield of per-check
 *                        failures otherwise
 *   sum            (u32) expected sum of msg(0)..msg(N-1)
 *                        = N * (N - 1) / 2 (triangular number)
 *   producer_errs  (u32) expected 0 (non-zero wait/post returns)
 *   consumer_errs  (u32) expected 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

static inline s32 syscall0_s32(u64 num)
{
    register u64 r3 __asm__("3") = 0;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline s32 syscall2_s32(u64 num, u64 a, u64 b)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

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

static inline s32 syscall8_s32(u64 num, u64 a, u64 b, u64 c, u64 d,
                               u64 e, u64 f, u64 g, u64 h)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r9 __asm__("9") = g;
    register u64 r10 __asm__("10") = h;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r8), "r"(r9), "r"(r10), "r"(r11)
        : "r0", "r12", "cr0", "ctr", "memory"
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

#define SYS_PPU_THREAD_EXIT   41
#define SYS_PPU_THREAD_JOIN   44
#define SYS_PPU_THREAD_CREATE 52
#define SYS_SEMAPHORE_CREATE  90
#define SYS_SEMAPHORE_WAIT    92
#define SYS_SEMAPHORE_POST    94

/* sys_semaphore_attribute_t: protocol@0 u32, pshared@4 u32,
 * ipc_key@8 u64, flags@16 s32, pad@20, name@24. The kernel
 * rejects a NULL attribute pointer with EFAULT and a protocol
 * that is neither FIFO (1) nor PRIORITY (2) with EINVAL (RPCS3
 * sys_semaphore.cpp sys_semaphore_create). */
static const struct {
    unsigned int protocol;      /* SYS_SYNC_FIFO */
    unsigned int pshared;       /* SYS_SYNC_NOT_PROCESS_SHARED */
    unsigned long long ipc_key;
    unsigned int flags;
    unsigned int pad;
    char name[8];
} sem_attr __attribute__((aligned(8))) = { 1, 0x200, 0, 0, 0, "" };

/* Syscall 52 takes 8 args: (thread_id*, param*, arg, unk, prio,
 * stacksize, flags, threadname*) per RPCS3 lv2.cpp /
 * sys_ppu_thread.cpp _sys_ppu_thread_create; liblv2's wrapper
 * passes unk = 0. The param* in r4 is a ppu_thread_param_t
 * { u32 entry_opd_ptr; u32 tls }, and the OPD it names is the
 * kernel's 8-byte { u32 code; u32 toc } form. The toolchain's
 * `&fn` resolves to the function's ELFv1 .opd descriptor -- 24
 * bytes of u64 fields -- so repack it before the syscall. */
struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr; /* -> opd_code below */
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

static struct cg_thread_param consumer_param __attribute__((aligned(8)));

/* N messages exchanged through a BUF_SIZE-slot bounded buffer.
 * N is larger than BUF_SIZE so the producer MUST block on the
 * space_sem at some point and the consumer MUST block on the
 * data_sem; this is the critical scheduling shape that exercises
 * real wake-and-resume. */
#define MESSAGES 32
#define BUF_SIZE 4

struct TestResult {
    unsigned int status;
    unsigned int sum;
    unsigned int producer_errs;
    unsigned int consumer_errs;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int buffer[BUF_SIZE] __attribute__((aligned(128)));
static unsigned int space_sem __attribute__((aligned(128)));
static unsigned int data_sem __attribute__((aligned(128)));
static volatile unsigned int consumer_sum __attribute__((aligned(128)));
static volatile unsigned int consumer_errs_shared __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

/* Consumer thread: for each of MESSAGES iterations, wait on
 * data_sem, read the slot, post space_sem. Accumulates the sum
 * into the shared `consumer_sum` word. */
static void consumer_entry(void *arg)
{
    (void)arg;
    unsigned int errs = 0;
    unsigned int local_sum = 0;
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 w = syscall2_s32(SYS_SEMAPHORE_WAIT, data_sem, 0);
        if (w != 0) {
            errs++;
            continue;
        }
        local_sum += buffer[i % BUF_SIZE];
        s32 p = syscall2_s32(SYS_SEMAPHORE_POST, space_sem, 1);
        if (p != 0)
            errs++;
    }
    consumer_sum = local_sum;
    consumer_errs_shared = errs;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xBEEFCAFE);
    for (;;) { }
}

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    result.status = status;
    result.sum = consumer_sum;
    result.producer_errs = 0xFFFFFFFF;
    result.consumer_errs = consumer_errs_shared;
    write_tty_result(&result);
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;
    unsigned int producer_errs = 0;

    consumer_sum = 0;
    consumer_errs_shared = 0xDEADBEEF;
    for (unsigned int i = 0; i < BUF_SIZE; i++)
        buffer[i] = 0;

    /* Create space_sem (initial BUF_SIZE, max BUF_SIZE). */
    ret = syscall4_s32(SYS_SEMAPHORE_CREATE,
        (unsigned long)&space_sem, (unsigned long)&sem_attr,
        BUF_SIZE, BUF_SIZE);
    if (ret != 0)
        return fail(0x01);
    if (space_sem == 0)
        return fail(0x02);

    /* Create data_sem (initial 0, max BUF_SIZE). */
    ret = syscall4_s32(SYS_SEMAPHORE_CREATE,
        (unsigned long)&data_sem, (unsigned long)&sem_attr,
        0, BUF_SIZE);
    if (ret != 0)
        return fail(0x04);
    if (data_sem == 0)
        return fail(0x08);

    /* Spawn consumer. */
    ret = syscall8_s32(
        SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
        make_thread_param(&consumer_param, (const void *)&consumer_entry),
        0,          /* arg */
        0,          /* unk (reserved; liblv2's wrapper passes 0) */
        1000,       /* prio */
        0x4000,     /* stacksize */
        0,          /* flags */
        0);         /* threadname (none) */
    if (ret != 0)
        return fail(0x10);

    /* Producer loop. */
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 w = syscall2_s32(SYS_SEMAPHORE_WAIT, space_sem, 0);
        if (w != 0) {
            producer_errs++;
            continue;
        }
        buffer[i % BUF_SIZE] = i;
        s32 p = syscall2_s32(SYS_SEMAPHORE_POST, data_sem, 1);
        if (p != 0)
            producer_errs++;
    }

    /* Join consumer. */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(0x20);
    if (retval != 0xBEEFCAFE)
        return fail(0x40);

    /* Expected sum: 0 + 1 + ... + (MESSAGES - 1) = N*(N-1)/2 */
    unsigned int expected_sum = (MESSAGES * (MESSAGES - 1)) / 2;

    result.status = 0;
    result.sum = consumer_sum;
    result.producer_errs = producer_errs;
    result.consumer_errs = consumer_errs_shared;
    if (result.sum != expected_sum)
        result.status |= 0x100;
    if (result.producer_errs != 0)
        result.status |= 0x200;
    if (result.consumer_errs != 0)
        result.status |= 0x400;
    write_tty_result(&result);
    return (int)result.status;
}
