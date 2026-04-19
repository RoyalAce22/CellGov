/* PPU program: two PPU threads, each writes to a disjoint memory
 * region, primary joins on the child and reports both values.
 *
 * Structural microtest for multi-PPU threading. It does not
 * exercise any sync primitive or atomic reservation contention.
 * It proves:
 *
 *   1. sys_ppu_thread_create successfully spawns a second PPU
 *      execution unit that the scheduler picks up.
 *   2. Both threads run (child reaches its write, primary does
 *      its own write without being starved).
 *   3. sys_ppu_thread_join blocks the primary until the child
 *      calls sys_ppu_thread_exit, and delivers the exit value.
 *   4. Both threads' writes to disjoint memory regions are
 *      visible to the primary after join.
 *
 * Uses direct syscalls only (no PSL1GHT HLE wrappers) so the
 * microtest can run on CellGov without loading liblv2.sprx --
 * matches the pattern of the other microtests in this corpus.
 *
 * Output layout (TTY-reported as CGOV magic + 16 bytes):
 *   status       (u32)  0 = pass, nonzero = per-step failure code
 *   child_word   (u32)  expected 0xAAAA_AAAA (child's write)
 *   parent_word  (u32)  expected 0xBBBB_BBBB (primary's write)
 *   join_retval  (u32)  expected 0xCAFE_F00D (child's exit value)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

/* Direct-syscall helpers. PSL1GHT provides LV2_SYSCALL inline
 * wrappers for many syscalls, but sys_ppu_thread_create (52) and
 * sys_ppu_thread_exit (41) are routed through HLE imports in the
 * library. We issue the syscalls directly so this microtest has
 * no HLE dependency. The calling convention is: r11 = syscall
 * number, r3-r10 = args, `sc` instruction, r3 = return value. */

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

#define SYS_PPU_THREAD_EXIT   41
#define SYS_PPU_THREAD_JOIN   44
#define SYS_PPU_THREAD_CREATE 52

struct TestResult {
    unsigned int status;
    unsigned int child_word;
    unsigned int parent_word;
    unsigned int join_retval;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int child_word __attribute__((aligned(128)));
static volatile unsigned int parent_word __attribute__((aligned(128)));
static struct TestResult result __attribute__((aligned(128)));

/* Child entry. Matches PSL1GHT's `void (*)(void *)` shape. Sets
 * one word and exits via direct syscall 41. */
static void child_entry(void *arg)
{
    (void)arg;
    child_word = 0xAAAAAAAA;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00D);
    /* unreachable */
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
    result.child_word = child_word;
    result.parent_word = parent_word;
    result.join_retval = 0;
    write_tty_result(&result);
    return (int)status;
}

int main(void)
{
    unsigned long long tid = 0;
    unsigned long long retval = 0;
    s32 ret;

    /* Poison the shared words so a partial write is detectable. */
    child_word = 0xDEADBEEF;
    parent_word = 0xDEADBEEF;

    /* On PPC64 ELFv1, a function pointer is already an OPD
     * pointer: the linker emits an OPD `{ code_addr, toc, env }`
     * for every function, and symbol references resolve to the
     * OPD's address. Passing `&child_entry` into the syscall
     * hands sys_ppu_thread_create the right pointer directly. */
    ret = syscall6_s32(
        SYS_PPU_THREAD_CREATE,
        (unsigned long)&tid,
        (unsigned long)&child_entry,
        0,
        1000,
        0x4000,
        0);
    if (ret != 0)
        return fail(1);

    /* Parent's own write -- disjoint from the child's. Happens
     * before the join so it is not gated by the child's exit. */
    parent_word = 0xBBBBBBBB;

    /* sys_ppu_thread_join(tid, &retval). */
    ret = syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval);
    if (ret != 0)
        return fail(2);

    /* Report the observed state. Any mismatch surfaces as a
     * non-zero status field via the TTY protocol. */
    result.status = 0;
    result.child_word = child_word;
    result.parent_word = parent_word;
    result.join_retval = (unsigned int)retval;

    if (result.child_word != 0xAAAAAAAA)
        result.status |= 0x10;
    if (result.parent_word != 0xBBBBBBBB)
        result.status |= 0x20;
    if (result.join_retval != 0xCAFEF00D)
        result.status |= 0x40;

    write_tty_result(&result);
    return (int)result.status;
}
