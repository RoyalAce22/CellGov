/* PPU program: two-SPU atomic contention driver.
 *
 * Launches two SPU threads in a single thread group. Each SPU runs
 * INCREMENTS_PER_THREAD CAS-retry increments on the shared atomic
 * line. After both complete, the final counter must be exactly
 * 2 * INCREMENTS_PER_THREAD (64 with the default), proving that
 * real atomic reservation contention across SPUs is modelled.
 *
 * Output (via TTY): status header + both SPU result slots.
 *   [0..16]   { status, final_counter_seen_by_spu1, retries_spu1, spe0 }
 *   [16..32]  { status, final_counter_seen_by_spu2, retries_spu2, spe1 }
 *   [32..48]  { 0, actual_final_counter, 0, 0 }
 *
 * Expected: actual_final_counter == 64.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

#define INCREMENTS_PER_THREAD 32

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_tty_tagged(const void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, data, len, &written);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    unsigned int buf[12] = { status, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
    write_tty_tagged(buf, 48);
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

/* 512-byte buffer, 256-byte aligned.
 * [0..16]:   SPU 0 result slot.
 * [16..32]:  SPU 1 result slot.
 * [32..48]:  summary slot (final counter).
 * [256..384]: atomic target (128-byte aligned).
 */
static unsigned char buf[512] __attribute__((aligned(256)));

/* Static globals so their addresses land in .bss (main-memory
 * view), not on the stack. The CellGov run-game memory view is
 * single-region main memory; stack addresses at 0xD0000000 are
 * not readable via read_committed. */
static sysSpuImage g_image;
static sys_spu_group_t g_group;
static sys_spu_thread_t g_t0;
static sys_spu_thread_t g_t1;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr0;
static sysSpuThreadAttribute g_thrattr1;
static sysSpuThreadArgument g_args0;
static sysSpuThreadArgument g_args1;
static unsigned int g_cause;
static unsigned int g_status;

int main(void)
{
    int ret;
    unsigned int *atomic_line = (unsigned int *)(buf + 256);

    memset(buf, 0xFF, 256);
    memset(buf + 256, 0, 128);
    atomic_line[0] = 0;

    ret = sysSpuImageOpen(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "atm_grp";
    ret = sysSpuThreadGroupCreate(&g_group, 2, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    memset(&g_thrattr0, 0, sizeof(g_thrattr0));
    g_thrattr0.nsize = 8;
    g_thrattr0.name = "atm_a";
    memset(&g_args0, 0, sizeof(g_args0));
    /* arg0 = EA of atomic line; arg1 = EA of result slot 0. */
    g_args0.arg0 = (u64)(unsigned long)(buf + 256);
    g_args0.arg1 = (u64)(unsigned long)(buf + 0);
    ret = sysSpuThreadInitialize(&g_t0, g_group, 0, &g_image, &g_thrattr0, &g_args0);
    if (ret != 0) return fail(3);

    memset(&g_thrattr1, 0, sizeof(g_thrattr1));
    g_thrattr1.nsize = 8;
    g_thrattr1.name = "atm_b";
    memset(&g_args1, 0, sizeof(g_args1));
    g_args1.arg0 = (u64)(unsigned long)(buf + 256);
    g_args1.arg1 = (u64)(unsigned long)(buf + 16);
    ret = sysSpuThreadInitialize(&g_t1, g_group, 1, &g_image, &g_thrattr1, &g_args1);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(5);

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(6);

    unsigned int *summary = (unsigned int *)(buf + 32);
    summary[0] = 0;
    summary[1] = atomic_line[0];
    summary[2] = 0;
    summary[3] = 0;

    write_tty_tagged(buf, 48);

    /* Encode the verdict into the process exit code: 0 on counter ==
     * 2 * INCREMENTS_PER_THREAD, 0x80 on mismatch so the CellGov-
     * side harness can read it via PROCESS_EXIT's code field
     * without needing to parse the TTY payload. */
    int verdict = (atomic_line[0] == (2u * INCREMENTS_PER_THREAD)) ? 0 : 0x80;

    sysSpuThreadGroupDestroy(g_group);
    sysSpuImageClose(&g_image);
    return verdict;
}
