/* PPU probe: a doubleword load and store that run across the seam
 * between two 64 KiB shared-memory segments mapped end to end.
 *
 * The probe allocates one 256 MiB area A of 64 KiB pages (330) and two
 * keyless 64 KiB segments M0 and M1 (332), and maps M0 at A and M1 at
 * A + 64K (334). The main thread fills the 16 bytes either side of the
 * seam with 0x10, 0x11, ... 0x2F by aligned word stores. A joinable
 * child thread then runs `ld` at A + 64K - 4, stores what it loaded,
 * runs `std` of 0x1122334455667788 at A + 64K - 4, and exits. A child
 * that faults is killed, and the main thread records that it never
 * passed the access. Every map is unmapped and every object freed
 * before the frame is written.
 *
 * Payload: big-endian u32 words.
 *    0  DONE ("SPAN") once the main thread finished, PROG in a
 *       progress record
 *    1  the step a progress record names (0 otherwise): 1 the setup,
 *       2 the child create, 3 the child start, 4 the wait for the
 *       child to reach the access, 5 the wait for it to pass both
 *    2  330 rc          3  A (low word)
 *    4  332 M0 rc       5  332 M1 rc
 *    6  334 M0 at A     7  334 M1 at A + 64K
 *    8  child create rc 9  child start rc
 *   10  reached flag   11  passed flag
 *   12  the ld result, high word   13  low word (0xDEADBEEF_DEADBEEF
 *       preset, left there when the child did not pass)
 *   14  get-priority rc on the child after the wait
 *   15  join rc, SKIP when the child did not pass
 *   16  the exit value the join returned, SKIP when it did not pass
 *   17..20  the words at A + 64K - 8, - 4, + 0, + 4 after the child
 *   21  335 A + 64K    22  335 A
 *   23  329 M1         24  329 M0
 *   25  331 A
 *   26..31  zero
 *
 * SKIP is 0xDEADD00D. A failed setup step leaves the later words SKIP.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT          41
#define SYS_PPU_THREAD_JOIN          44
#define SYS_PPU_THREAD_GET_PRIORITY  48
#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_64K  0x200
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define K64       0x10000ULL

#define WORDS    32
#define DONE     0x5350414Eu /* "SPAN" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define SKIP     0xDEADD00Du

#define THREAD_JOINABLE 1
#define CHILD_EXIT  0xE5
#define CHILD_PRIO  1000
#define STACK_SIZE  0x4000
#define BEFORE_TICKS 80000000ull  /* about one second of time base */
#define AFTER_TICKS  160000000ull /* about two seconds */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile unsigned int before;
static volatile unsigned int after;
static volatile unsigned long long loaded;
static volatile unsigned long access_ea;

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

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
    unsigned long long stored = 0x1122334455667788ull;
    unsigned long ea = access_ea;
    (void)arg;
    before = 1;
    __sync_synchronize();
    __asm__ volatile("ld %0, 0(%1)\n\t" : "+r"(value) : "b"(ea) : "memory");
    loaded = value;
    __asm__ volatile("std %0, 0(%1)\n\t" : : "r"(stored), "b"(ea) : "memory");
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

static u64 unmap(u64 addr)
{
    w32 = 0xFFFFFFFFu;
    return sc(SYS_MMAPPER_UNMAP_SHM, addr, EA(&w32), 0, 0);
}

int main(void)
{
    unsigned int i, map0, map1, m0_ok, m1_ok;
    u64 a = 0, m0 = 0, m1 = 0;
    volatile unsigned int *seam;

    for (i = 0; i < WORDS; i++)
        result[i] = i < 26 ? SKIP : 0;
    before = after = 0;
    loaded = 0xDEADBEEFDEADBEEFull;
    progress(1);

    w32 = 0xFFFFFFFFu;
    result[2] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    a = w32;
    result[3] = (unsigned int)a;
    if (result[2] != 0)
        goto done;
    w32 = 0xFFFFFFFFu;
    result[4] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m0 = w32;
    m0_ok = result[4] == 0;
    w32 = 0xFFFFFFFFu;
    result[5] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m1 = w32;
    m1_ok = result[5] == 0;
    map0 = map1 = 0;
    if (m0_ok && m1_ok) {
        result[6] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a, m0, PROT_RW, 0);
        map0 = result[6] == 0;
        if (map0) {
            result[7] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a + K64, m1, PROT_RW, 0);
            map1 = result[7] == 0;
        }
    }

    if (map0 && map1) {
        seam = (volatile unsigned int *)(unsigned long)(a + K64 - 16);
        for (i = 0; i < 8; i++) {
            unsigned int b = 0x10 + 4 * i;
            seam[i] = (b << 24) | ((b + 1) << 16) | ((b + 2) << 8) | (b + 3);
        }
        access_ea = (unsigned long)(a + K64 - 4);

        progress(2);
        result[8] = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE, EA(&tid),
                                               make_param(&param, (const void *)&child), 0, 0,
                                               CHILD_PRIO, STACK_SIZE, THREAD_JOINABLE, 0);
        if (result[8] == 0) {
            progress(3);
            result[9] = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid, 0);
        }
        if (result[8] == 0 && result[9] == 0) {
            progress(4);
            result[10] = wait_flag(&before, BEFORE_TICKS);
            progress(5);
            result[11] = wait_flag(&after, AFTER_TICKS);
            __sync_synchronize();
            result[12] = (unsigned int)(loaded >> 32);
            result[13] = (unsigned int)loaded;
            result[14] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio));
            if (result[11]) {
                exit_value = 0xFFFFFFFFFFFFFFFFull;
                result[15] = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, EA(&exit_value));
                result[16] = (unsigned int)exit_value;
            }
        }
        result[17] = seam[2];
        result[18] = seam[3];
        result[19] = seam[4];
        result[20] = seam[5];
    }

    if (map1)
        result[21] = (unsigned int)unmap(a + K64);
    if (map0)
        result[22] = (unsigned int)unmap(a);
    if (m1_ok)
        result[23] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0);
    if (m0_ok)
        result[24] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m0, 0, 0, 0);
    result[25] = (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0);

done:
    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
