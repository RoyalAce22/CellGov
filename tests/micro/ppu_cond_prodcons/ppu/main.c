/* PPU program: producer-consumer bounded buffer guarded by a
 * heavy mutex and two condition variables. Producer thread
 * deposits N messages into a circular buffer; consumer thread
 * reads them and sums. Proves sys_cond_wait / _signal semantics
 * including the drop-mutex-on-wait / re-acquire-on-wake two-hop
 * protocol.
 *
 * Structural microtest for the cond primitive. It proves:
 *
 *   1. sys_cond_create writes the id and binds to an existing
 *      heavy mutex.
 *   2. sys_cond_wait on a cond whose predicate fails (count ==
 *      0 for consumer, count == BUF_SIZE for producer) blocks
 *      the caller AND observably releases the mutex so the
 *      counterparty can enter the critical section.
 *   3. sys_cond_signal wakes the parked waiter, which
 *      re-acquires the mutex before returning CELL_OK.
 *   4. A bounded-buffer exchange with N messages passes every
 *      message through without loss across arbitrary scheduler
 *      interleavings.
 *
 * Bounded-buffer protocol:
 *   - mutex:        guards head, tail, count, buffer
 *   - not_empty:    consumer waits when count == 0
 *   - not_full:     producer waits when count == BUF_SIZE
 *
 * Output layout (TTY: CGOV magic + 16 bytes):
 *   status         (u32) 0 = pass, bitfield of per-check
 *                        failures otherwise
 *   sum            (u32) expected N*(N-1)/2
 *   producer_errs  (u32) expected 0
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

static inline s32 syscall1_s32(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
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

static inline s32 syscall3_s32(u64 num, u64 a, u64 b, u64 c)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r11)
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
#define SYS_MUTEX_CREATE     100
#define SYS_MUTEX_LOCK       102
#define SYS_MUTEX_UNLOCK     104
#define SYS_COND_CREATE      105
#define SYS_COND_WAIT        107
#define SYS_COND_SIGNAL      108

/* sys_mutex_attribute_t: protocol@0 u32, recursive@4 u32,
 * pshared@8 u32, adaptive@12 u32, ipc_key@16 u64, flags@24 s32,
 * pad@28, name@32. The kernel rejects a NULL attribute pointer
 * with EFAULT, and recursive must be SYS_SYNC_RECURSIVE (0x10)
 * or SYS_SYNC_NOT_RECURSIVE (0x20) (RPCS3 sys_mutex.cpp
 * sys_mutex_create). */
static const struct {
    unsigned int protocol;      /* SYS_SYNC_FIFO */
    unsigned int recursive;     /* SYS_SYNC_NOT_RECURSIVE */
    unsigned int pshared;       /* SYS_SYNC_NOT_PROCESS_SHARED */
    unsigned int adaptive;      /* SYS_SYNC_NOT_ADAPTIVE */
    unsigned long long ipc_key;
    int flags;
    unsigned int pad;
    char name[8];
} mutex_attr __attribute__((aligned(8))) =
    { 1, 0x20, 0x200, 0x2000, 0, 0, 0, "" };

/* sys_cond_attribute_t: pshared@0 u32, flags@4 s32, ipc_key@8
 * u64, name@16. The kernel reads pshared / flags / ipc_key out
 * of the struct, so the pointer must name valid memory (RPCS3
 * sys_cond.cpp sys_cond_create). */
static const struct {
    unsigned int pshared;       /* SYS_SYNC_NOT_PROCESS_SHARED */
    int flags;
    unsigned long long ipc_key;
    char name[8];
} cond_attr __attribute__((aligned(8))) = { 0x200, 0, 0, "" };

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
 * N larger than BUF_SIZE forces both sides to block on cond at
 * some point; this is the shape that exercises the drop-mutex-
 * on-wait / re-acquire-on-wake path. */
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
static volatile unsigned int head_idx __attribute__((aligned(128)));
static volatile unsigned int tail_idx __attribute__((aligned(128)));
static volatile unsigned int count __attribute__((aligned(128)));

static unsigned int mutex_id __attribute__((aligned(128)));
static unsigned int cond_not_empty __attribute__((aligned(128)));
static unsigned int cond_not_full __attribute__((aligned(128)));
static volatile unsigned int consumer_sum __attribute__((aligned(128)));
static volatile unsigned int consumer_errs_shared __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

/* Consumer thread: for each of MESSAGES iterations, lock mutex,
 * wait on not_empty if count == 0, drain one message from
 * buffer[head], signal not_full, unlock. */
static void consumer_entry(void *arg)
{
    (void)arg;
    unsigned int errs = 0;
    unsigned int local_sum = 0;
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 r = syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
        if (r != 0) {
            errs++;
            continue;
        }
        while (count == 0) {
            s32 w = syscall2_s32(SYS_COND_WAIT, cond_not_empty, 0);
            if (w != 0) {
                errs++;
                break;
            }
        }
        local_sum += buffer[head_idx];
        head_idx = (head_idx + 1) % BUF_SIZE;
        count--;
        s32 s = syscall1_s32(SYS_COND_SIGNAL, cond_not_full);
        if (s != 0)
            errs++;
        s32 u = syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
        if (u != 0)
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
    head_idx = 0;
    tail_idx = 0;
    count = 0;
    for (unsigned int i = 0; i < BUF_SIZE; i++)
        buffer[i] = 0;

    /* Create mutex. */
    ret = syscall2_s32(SYS_MUTEX_CREATE,
        (unsigned long)&mutex_id, (unsigned long)&mutex_attr);
    if (ret != 0)
        return fail(0x01);
    if (mutex_id == 0)
        return fail(0x02);

    /* Create cond_not_empty bound to the mutex. */
    ret = syscall3_s32(SYS_COND_CREATE,
        (unsigned long)&cond_not_empty, mutex_id,
        (unsigned long)&cond_attr);
    if (ret != 0)
        return fail(0x04);
    if (cond_not_empty == 0)
        return fail(0x08);

    /* Create cond_not_full bound to the mutex. */
    ret = syscall3_s32(SYS_COND_CREATE,
        (unsigned long)&cond_not_full, mutex_id,
        (unsigned long)&cond_attr);
    if (ret != 0)
        return fail(0x10);
    if (cond_not_full == 0)
        return fail(0x20);

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
        return fail(0x40);

    /* Producer loop. */
    for (unsigned int i = 0; i < MESSAGES; i++) {
        s32 r = syscall2_s32(SYS_MUTEX_LOCK, mutex_id, 0);
        if (r != 0) {
            producer_errs++;
            continue;
        }
        while (count == BUF_SIZE) {
            s32 w = syscall2_s32(SYS_COND_WAIT, cond_not_full, 0);
            if (w != 0) {
                producer_errs++;
                break;
            }
        }
        buffer[tail_idx] = i;
        tail_idx = (tail_idx + 1) % BUF_SIZE;
        count++;
        s32 s = syscall1_s32(SYS_COND_SIGNAL, cond_not_empty);
        if (s != 0)
            producer_errs++;
        s32 u = syscall1_s32(SYS_MUTEX_UNLOCK, mutex_id);
        if (u != 0)
            producer_errs++;
    }

    /* Join consumer. */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(0x80);
    if (retval != 0xBEEFCAFE)
        return fail(0x100);

    unsigned int expected_sum = (MESSAGES * (MESSAGES - 1)) / 2;

    result.status = 0;
    result.sum = consumer_sum;
    result.producer_errs = producer_errs;
    result.consumer_errs = consumer_errs_shared;
    if (result.sum != expected_sum)
        result.status |= 0x1000;
    if (result.producer_errs != 0)
        result.status |= 0x2000;
    if (result.consumer_errs != 0)
        result.status |= 0x4000;
    write_tty_result(&result);
    return (int)result.status;
}
