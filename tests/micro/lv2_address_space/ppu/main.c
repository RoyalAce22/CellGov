/* PPU probe: where the user address space puts what a game finds in it
 * -- the pages below the image, the primary and child thread stacks, and
 * what an RSX context maps -- read with sys_memory_get_page_attribute
 * (351) and sys_ppu_thread_get_stack_information (49).
 *
 * Four joinable child threads with distinct stack sizes each record their
 * r1 and their own 49 answer, then exit; main joins each. After rsxInit
 * the probe reads one word of RSX local memory, writes a word there and
 * reads it back, then sweeps two ranges for page-attribute runs.
 *
 * Payload (big-endian u32 words; SKIP = 0x5C1F5C1F):
 *   0  failed step (0 = none)   1  its return code
 *   2 + 3k, page k (0-4: 0x0, 0x8000, 0xF000, 0x10000, 0x40000000, before
 *       any RSX call): 351 rc, attribute (low word), page size
 *  17  main's r1   18  49 rc   19  stack address   20  stack size
 *  21 + 5n, child n (0-3, stack 0x4000, 0x8000, 0x10000, 0x40000): 52 rc,
 *       r1, 49 rc, stack address, stack size
 *  41  rsxInit rc   42  local address   43  local size   44  IO address
 *  45  351 rc at the local address   46  attribute   47  page size
 *  48  local word at +0x10000 as read   49  the same word after writing
 *       0xC0DE1569 there
 *  50  run count of the sweeps after rsxInit: 0x40000000-0x50000000 and
 *       0xC0000000-0xE0000000, 64 KiB steps
 *  51 + 4r, run r (0-11): start, end, attribute, page size
 */

#include <string.h>

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 99

#define SYS_PPU_THREAD_EXIT         41
#define SYS_PPU_THREAD_JOIN         44
#define SYS_PPU_THREAD_GET_STACK    49
#define SYS_MEMORY_GET_PAGE_ATTR    351

#define THREAD_JOINABLE 1
#define CHILD_PRIO  1000
#define CHILDREN    4
#define RUNS        12

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000
#define LOCAL_PROBE 0x10000
#define LOCAL_MARK  0xC0DE1569u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static const unsigned int child_stack[CHILDREN] = { 0x4000, 0x8000, 0x10000, 0x40000 };
static const unsigned long long early_page[5] = {
    0x0ull, 0x8000ull, 0xF000ull, 0x10000ull, 0x40000000ull,
};

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char scratch[64] __attribute__((aligned(128)));
static volatile unsigned int stack_info[CHILDREN + 1][2] __attribute__((aligned(8)));
static volatile unsigned int child_r1[CHILDREN];
static volatile unsigned int child_rc[CHILDREN];
static gcmContextData *context;

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
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result[0] = step;
    result[1] = (unsigned int)rc;
    write_frame();
    return (int)step;
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

static unsigned int self_r1(void)
{
    unsigned long r1;
    __asm__ volatile ("mr %0, 1" : "=r"(r1));
    return (unsigned int)r1;
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

/* Child n records its r1 and its own stack information, then exits. */
static void child(u64 n)
{
    child_r1[n] = self_r1();
    child_rc[n] = (unsigned int)syscall1_s32(SYS_PPU_THREAD_GET_STACK,
                                             (u64)(unsigned long)&stack_info[n + 1][0]);
    __sync_synchronize();
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* 351 on `addr`: rc, attribute low word and page size into result[at..]. */
static void page_attr(unsigned int at, unsigned long long addr)
{
    unsigned int i;
    for (i = 0; i < sizeof(scratch); i++)
        scratch[i] = 0;
    result[at] = (unsigned int)syscall2_s32(SYS_MEMORY_GET_PAGE_ATTR, addr,
                                            (u64)(unsigned long)scratch);
    result[at + 1] = *(volatile unsigned int *)(scratch + 4);
    result[at + 2] = *(volatile unsigned int *)(scratch + 16);
}

static unsigned int nruns;

/* Merge every mapped 64 KiB step of [from, to) into runs of one
 * attribute and one page size, appended after any earlier sweep's. */
static void sweep(unsigned long long from, unsigned long long to)
{
    unsigned long long addr, run_start = 0;
    unsigned int run_attr = 0, run_page = 0, in_run = 0;
    for (addr = from; addr <= to; addr += 0x10000ull) {
        unsigned int rc = 1, attr = 0, page = 0, i;
        if (addr < to) {
            for (i = 0; i < sizeof(scratch); i++)
                scratch[i] = 0;
            rc = (unsigned int)syscall2_s32(SYS_MEMORY_GET_PAGE_ATTR, addr,
                                            (u64)(unsigned long)scratch);
            attr = *(volatile unsigned int *)(scratch + 4);
            page = *(volatile unsigned int *)(scratch + 16);
        }
        if (in_run && (rc != 0 || attr != run_attr || page != run_page)) {
            if (nruns < RUNS) {
                unsigned int e = 51 + 4 * nruns;
                result[e] = (unsigned int)run_start;
                result[e + 1] = (unsigned int)addr;
                result[e + 2] = run_attr;
                result[e + 3] = run_page;
            }
            nruns++;
            in_run = 0;
        }
        if (!in_run && rc == 0) {
            in_run = 1;
            run_start = addr;
            run_attr = attr;
            run_page = page;
        }
    }
}

static struct thread_param params[CHILDREN] __attribute__((aligned(8)));

int main(void)
{
    unsigned int n, rc;
    unsigned long long tid[CHILDREN], value;
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    volatile unsigned int *word;

    memset((void *)result, 0, sizeof(result));

    /* Pages, before any RSX call. */
    for (n = 0; n < 5; n++)
        page_attr(2 + 3 * n, early_page[n]);

    /* The primary stack. */
    result[17] = self_r1();
    result[18] = (unsigned int)syscall1_s32(SYS_PPU_THREAD_GET_STACK,
                                            (u64)(unsigned long)&stack_info[0][0]);
    result[19] = stack_info[0][0];
    result[20] = stack_info[0][1];

    /* Child stacks. */
    for (n = 0; n < CHILDREN; n++) {
        rc = (unsigned int)syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE,
                                        (u64)(unsigned long)&tid[n],
                                        make_param(&params[n], (const void *)&child), n, 0,
                                        CHILD_PRIO, child_stack[n], THREAD_JOINABLE, 0);
        result[21 + 5 * n] = rc;
        if (rc != 0)
            return fail(1 + n, (int)rc);
        rc = (unsigned int)syscall2_s32(CGOV_SYS_PPU_THREAD_START, tid[n], 0);
        if (rc != 0)
            return fail(5 + n, (int)rc);
        rc = (unsigned int)syscall2_s32(SYS_PPU_THREAD_JOIN, tid[n], (u64)(unsigned long)&value);
        if (rc != 0)
            return fail(9 + n, (int)rc);
        result[22 + 5 * n] = child_r1[n];
        result[23 + 5 * n] = child_rc[n];
        result[24 + 5 * n] = stack_info[n + 1][0];
        result[25 + 5 * n] = stack_info[n + 1][1];
    }

    /* RSX. */
    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0)
        return fail(13, -1);
    rc = (unsigned int)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    result[41] = rc;
    if (rc != 0)
        return fail(14, (int)rc);
    gcmGetConfiguration(&config);
    memcpy((void *)&result[42], &config.localAddress, 4);
    result[43] = config.localSize;
    memcpy((void *)&result[44], &config.ioAddress, 4);
    page_attr(45, (unsigned long long)result[42]);
    word = (volatile unsigned int *)(unsigned long)(result[42] + LOCAL_PROBE);
    result[48] = *word;
    *word = LOCAL_MARK;
    __sync_synchronize();
    result[49] = *word;

    sweep(0x40000000ull, 0x50000000ull);
    sweep(0xC0000000ull, 0xE0000000ull);
    result[50] = nruns;

    write_frame();
    return 0;
}
