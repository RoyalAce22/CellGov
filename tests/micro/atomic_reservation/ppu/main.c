/* PPU program: atomic reservation (getllar / putllc).
 *
 * Launches an SPU thread that atomically loads a 128-byte cache line
 * (getllar), overwrites it with 0xBBBBBBBB, and conditionally stores
 * it back (putllc).  No contention exists, so the conditional store
 * should succeed.
 *
 * Buffer layout (512 bytes, 256-byte aligned):
 *   +0:    u32 status  (0 = putllc succeeded)
 *   +4:    u32 atomic_stat (raw MFC_RdAtomicStat, expect 0)
 *   +8:    padding (8 bytes)
 *   +16:   u8[128] data copy (SPU DMA puts its LS copy here)
 *   +144:  padding
 *   +256:  u8[128] atomic target (getllar/putllc target, initially 0xAA)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

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
    unsigned int buf[2] = { status, 0 };
    write_tty_tagged(buf, 8);
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

/* 512-byte buffer, 256-byte aligned.
 * [0..144]:   result area (status header + data copy)
 * [256..384]: atomic target (128-byte aligned for getllar/putllc)
 */
static unsigned char buf[512] __attribute__((aligned(256)));

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause, status;

    /* Poison the result area. */
    memset(buf, 0xFF, 256);

    /* Initialize atomic target with known pattern. */
    memset(buf + 256, 0xAA, 128);

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "atm_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "atm_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(5);

    /* Output status header (16 bytes) + data copy (128 bytes). */
    write_tty_tagged(buf, 16 + 128);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
