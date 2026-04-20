/* PPU program: two PPU threads increment a shared counter via
 * lwarx / stwcx retry loops. Proves real atomic reservation
 * contention semantics:
 *
 *   1. Lwarx loads the counter and installs a reservation on the
 *      enclosing 128-byte line.
 *   2. Stwcx stores the incremented value only if the reservation
 *      is still held (another unit's committed write to the line
 *      would clear it). On failure (CR0.EQ=0) the thread loops
 *      back to the lwarx to retry.
 *   3. The retry loop eventually wins the CAS and the increment
 *      commits.
 *
 * Without real reservation contention (an always-succeed stwcx)
 * both threads could "succeed" concurrently and drop updates --
 * the final counter would be less than 2*N. With real contention
 * every successful stwcx is atomic with its lwarx, so the final
 * counter is exactly 2*N regardless of the scheduler's
 * interleaving.
 *
 * Output layout (TTY-reported as CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass, bitfield of per-check
 *                         failures otherwise
 *   counter        (u32)  expected 2*N
 *   parent_retries (u32)  informational: stwcx failures on the
 *                         main thread. Not an error by itself;
 *                         any positive value is proof contention
 *                         occurred.
 *   child_retries  (u32)  same, on the child thread.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

/* Direct-syscall helpers for thread lifecycle (no HLE). Same
 * pattern as the ppu_lwmutex_counter / ppu_two_threads_disjoint
 * microtests. */

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

/* Each thread performs INCREMENTS_PER_THREAD successful CAS
 * increments. Kept small so the test completes in O(100K) guest
 * instructions across all interleavings. */
#define INCREMENTS_PER_THREAD 64

struct TestResult {
    unsigned int status;
    unsigned int counter;
    unsigned int parent_retries;
    unsigned int child_retries;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

/* Shared counter plus per-thread retry counters. 128-byte-aligned
 * so the counter has its own cache line; the retry counters are
 * thread-private and go on separate lines to avoid false sharing
 * that would muddy the reservation semantics under test. */
static volatile unsigned int counter __attribute__((aligned(128)));
static volatile unsigned int child_retries __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

/* Atomic increment via lwarx / stwcx retry. Returns the number of
 * failed stwcx attempts before the successful one (0 = first-try
 * win). */
static unsigned int atomic_increment(volatile unsigned int *addr)
{
    unsigned int retries = 0;
    unsigned int tmp;
    unsigned int cr_eq;
    for (;;) {
        __asm__ volatile (
            "lwarx  %0, 0, %2\n"
            "addi   %0, %0, 1\n"
            "stwcx. %0, 0, %2\n"
            "mfcr   %1\n"
            : "=&r"(tmp), "=r"(cr_eq)
            : "r"(addr)
            : "cc", "memory"
        );
        /* stwcx. sets CR0 EQ on success. CR0 occupies bits 0..3 of
         * CR (field 0, highest-order). EQ is bit 2 of that field,
         * which is bit 2 counting from MSB of the full CR, i.e.
         * bit 29 (31-2) when viewed as a 32-bit LSB-indexed value.
         * mfcr copies CR verbatim to a GPR, so we test that bit. */
        if (cr_eq & (1u << 29))
            return retries;
        retries++;
    }
}

static void child_entry(void *arg)
{
    (void)arg;
    unsigned int i;
    unsigned int total_retries = 0;
    for (i = 0; i < INCREMENTS_PER_THREAD; i++) {
        total_retries += atomic_increment(&counter);
    }
    child_retries = total_retries;
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
    result.parent_retries = 0;
    result.child_retries = child_retries;
    write_tty_result(&result);
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;
    unsigned int i;
    unsigned int parent_retries = 0;

    counter = 0;
    child_retries = 0xDEADBEEF;

    /* Spawn the child thread. */
    ret = syscall6_s32(
        SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
        (unsigned long)&child_entry,
        0,
        1000,
        0x4000,
        0);
    if (ret != 0)
        return fail(0x04);

    /* Primary thread's own N CAS-increment cycles. Races against
     * the child's identical loop. */
    for (i = 0; i < INCREMENTS_PER_THREAD; i++) {
        parent_retries += atomic_increment(&counter);
    }

    /* Join the child to ensure its increments finished. */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(0x08);
    if (retval != 0xCAFEF00D)
        return fail(0x10);

    /* Report the observed state. */
    result.counter = counter;
    result.parent_retries = parent_retries;
    result.child_retries = child_retries;
    result.status = 0;
    if (result.counter != (unsigned int)(2 * INCREMENTS_PER_THREAD))
        result.status |= 0x20;
    write_tty_result(&result);
    return (int)result.status;
}
