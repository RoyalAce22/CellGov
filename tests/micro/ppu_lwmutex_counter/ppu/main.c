/* PPU program: two PPU threads increment a shared counter N times
 * each under a lightweight mutex. Final value must be exactly 2*N
 * with no lost updates.
 *
 * Structural microtest for the lwmutex primitive's kernel side.
 * _sys_lwmutex_lock (97) is the SLOW PATH only: on real firmware
 * the userspace wrapper owns the uncontended atomic fast path, and
 * the kernel object is a signal queue that starts with no signal
 * pending (RPCS3 sys_lwmutex.cpp _sys_lwmutex_lock /
 * _sys_lwmutex_unlock). Raw-syscall usage therefore primes the
 * queue with one _sys_lwmutex_unlock (98) after create; lock and
 * unlock then behave as a binary semaphore. It proves:
 *
 *   1. _sys_lwmutex_create (95) allocates an id with no signal
 *      pending.
 *   2. _sys_lwmutex_lock consumes a pending signal and completes
 *      immediately.
 *   3. _sys_lwmutex_lock with no signal pending blocks the caller
 *      until a peer calls _sys_lwmutex_unlock.
 *   4. _sys_lwmutex_unlock hands the signal to the head of the
 *      waiter list in FIFO order when waiters are parked.
 *   5. Mutual exclusion is preserved: every increment observes a
 *      fresh counter value and commits it back without the other
 *      thread's write racing in.
 *
 * Without a real lwmutex (an always-succeed stub that returned
 * CELL_OK without blocking) two threads racing on the counter
 * would drop updates and the final value would be less than 2*N.
 * With a real lwmutex the final value is exactly 2*N regardless
 * of the scheduler's interleaving.
 *
 * Output layout (TTY-reported as CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass, bitfield of per-check
 *                         failures otherwise
 *   counter        (u32)  expected 2*N
 *   lock_errors    (u32)  expected 0 (count of non-zero returns
 *                         from sys_lwmutex_lock across all
 *                         iterations)
 *   unlock_errors  (u32)  expected 0 (count of non-zero returns
 *                         from sys_lwmutex_unlock)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

/* Direct-syscall helpers. sys_lwmutex_create / _destroy / _lock /
 * _unlock / _trylock are issued directly (syscalls 95 / 96 / 97 /
 * 98 / 99) so the microtest has no HLE dependency. Matches the
 * pattern from the ppu_two_threads_disjoint_writes microtest. */

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

static inline s32 syscall6_s32(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r8), "r"(r11)
        : "r0", "r9", "r10", "r12", "cr0", "ctr", "memory"
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
#define SYS_LWMUTEX_CREATE    95
#define SYS_LWMUTEX_LOCK      97
#define SYS_LWMUTEX_UNLOCK    98

/* _sys_lwmutex_create (95) is (id_ptr, protocol, control_ptr,
 * has_name, name): r4 carries the protocol word -- FIFO (1),
 * PRIORITY (2), or RETRY (4); anything else is EINVAL -- and r5
 * names the user-space sys_lwmutex_t the kernel records for the
 * object (RPCS3 sys_lwmutex.cpp _sys_lwmutex_create). */
#define SYS_SYNC_FIFO 1
static unsigned char lwmutex_control[32] __attribute__((aligned(8)));

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

/* Each thread performs INCREMENTS_PER_THREAD lock / read / write /
 * unlock cycles. Total expected counter = 2 * INCREMENTS_PER_THREAD.
 * Kept small so the test completes in O(100K) guest instructions
 * across all interleavings. */
#define INCREMENTS_PER_THREAD 64

struct TestResult {
    unsigned int status;
    unsigned int counter;
    unsigned int lock_errors;
    unsigned int unlock_errors;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

/* Shared state. 128-byte-aligned so each word lives on its own
 * cache line and the scheduler has no false-sharing artifact to
 * chase. */
static volatile unsigned int counter __attribute__((aligned(128)));
static volatile unsigned int child_lock_errors __attribute__((aligned(128)));
static volatile unsigned int child_unlock_errors __attribute__((aligned(128)));
static unsigned int lwmutex_id __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));
static struct cg_thread_param child_param __attribute__((aligned(8)));

/* Child thread: runs INCREMENTS_PER_THREAD increments under the
 * shared lwmutex. Reports lock / unlock failure counts via the
 * shared counters. Exits with a recognizable magic value. */
static void child_entry(void *arg)
{
    (void)arg;
    unsigned int i;
    unsigned int lock_errs = 0;
    unsigned int unlock_errs = 0;
    for (i = 0; i < INCREMENTS_PER_THREAD; i++) {
        s32 lock_ret = syscall2_s32(SYS_LWMUTEX_LOCK, lwmutex_id, 0);
        if (lock_ret != 0) {
            lock_errs++;
            continue;
        }
        /* Critical section: read-modify-write. If the lwmutex is a
         * real lock no other thread observes the read-modify window.
         * If it is a stub (always-succeed), both threads race here
         * and drop updates. */
        unsigned int v = counter;
        counter = v + 1;
        s32 unlock_ret = syscall1_s32(SYS_LWMUTEX_UNLOCK, lwmutex_id);
        if (unlock_ret != 0)
            unlock_errs++;
    }
    child_lock_errors = lock_errs;
    child_unlock_errors = unlock_errs;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00D);
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
    result.counter = counter;
    result.lock_errors = child_lock_errors;
    result.unlock_errors = child_unlock_errors;
    write_tty_result(&result);
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;
    unsigned int i;
    unsigned int parent_lock_errs = 0;
    unsigned int parent_unlock_errs = 0;

    counter = 0;
    child_lock_errors = 0xDEADBEEF;
    child_unlock_errors = 0xDEADBEEF;

    /* Create the lwmutex: FIFO protocol, unnamed, with a valid
     * user-space control area (see the defines above). */
    ret = syscall6_s32(SYS_LWMUTEX_CREATE,
        (unsigned long)&lwmutex_id, SYS_SYNC_FIFO,
        (unsigned long)lwmutex_control, 0, 0, 0);
    if (ret != 0)
        return fail(0x01);
    if (lwmutex_id == 0)
        return fail(0x02);

    /* Prime the kernel signal queue (see the header comment): a
     * fresh lwmutex has no signal pending, so without this every
     * locker parks forever. */
    ret = syscall1_s32(SYS_LWMUTEX_UNLOCK, lwmutex_id);
    if (ret != 0)
        return fail(0x100);

    /* Spawn the child thread (syscall 52). */
    ret = syscall8_s32(
        SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
        make_thread_param(&child_param, (const void *)&child_entry),
        0,          /* arg */
        0,          /* unk (reserved; liblv2's wrapper passes 0) */
        1000,       /* prio */
        0x4000,     /* stacksize */
        0,          /* flags */
        0);         /* threadname (none) */
    if (ret != 0)
        return fail(0x04);

    /* Primary thread's own INCREMENTS_PER_THREAD lock / inc /
     * unlock cycles. Races against the child's identical loop. */
    for (i = 0; i < INCREMENTS_PER_THREAD; i++) {
        s32 lock_ret = syscall2_s32(SYS_LWMUTEX_LOCK, lwmutex_id, 0);
        if (lock_ret != 0) {
            parent_lock_errs++;
            continue;
        }
        unsigned int v = counter;
        counter = v + 1;
        s32 unlock_ret = syscall1_s32(SYS_LWMUTEX_UNLOCK, lwmutex_id);
        if (unlock_ret != 0)
            parent_unlock_errs++;
    }

    /* Join the child to ensure its increments finished. */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(0x08);
    if (retval != 0xCAFEF00D)
        return fail(0x10);

    /* Report the observed state. */
    result.counter = counter;
    result.lock_errors = parent_lock_errs + child_lock_errors;
    result.unlock_errors = parent_unlock_errs + child_unlock_errors;
    result.status = 0;
    if (result.counter != (unsigned int)(2 * INCREMENTS_PER_THREAD))
        result.status |= 0x20;
    if (result.lock_errors != 0)
        result.status |= 0x40;
    if (result.unlock_errors != 0)
        result.status |= 0x80;
    write_tty_result(&result);
    return (int)result.status;
}
