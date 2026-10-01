/* PPU program: store into a lock line an SPU holds a reservation on.
 *
 * Starts one SPU thread with the address of a 512-byte block, waits
 * for the SPU to set the block's ready flag, then stores 0x5A5A5A5A
 * into the block's lock line A. The SPU waits on the lock-line
 * reservation lost event and records what it saw. After join the PPU
 * outputs the 32-byte result at the block's start; spu/main.c gives
 * its layout.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"

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
    CGOV_OUT_FILE_WRITE(data, len);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    unsigned int buf[2] = { status, 0 };
    write_tty_tagged(buf, 8);
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

/* +0 result, +128 line A, +256 line B, +384 ready flag. */
static volatile unsigned int block[128] __attribute__((aligned(128)));

static sysSpuImage g_image;
static sys_spu_group_t g_group;
static sys_spu_thread_t g_thread;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;
static unsigned int g_cause;
static unsigned int g_status;

int main(void)
{
    int ret;

    memset((void *)block, 0, sizeof(block));
    memset((void *)block, 0xFF, 32);

    ret = sysSpuImageOpen(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 7;
    g_grpattr.name = "lrevent";
    ret = sysSpuThreadGroupCreate(&g_group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    /* PSL1GHT's argN fields land in SPU r(3+N): arg1 is the block. */
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 7;
    g_thrattr.name = "lrevent";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)block;
    ret = sysSpuThreadInitialize(&g_thread, g_group, 0, &g_image,
                                 &g_thrattr, &g_args);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(4);

    while (block[96] == 0) {
    }
    __sync_synchronize();
    block[32] = 0x5A5A5A5A;
    __sync_synchronize();

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(5);

    write_tty_tagged((const void *)block, 32);

    sysSpuThreadGroupDestroy(g_group);
    return 0;
}
