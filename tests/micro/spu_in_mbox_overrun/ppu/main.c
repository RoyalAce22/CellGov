/* PPU program: the PPE overruns the SPU inbound mailbox.
 *
 * Starts one SPU thread, writes five messages (0x11 .. 0x55) into its
 * four-entry inbound mailbox with sys_spu_thread_write_spu_mb, then sets
 * a flag the SPU polls. The SPU reads the mailbox count and four
 * messages. After join, the PPU outputs the 32-byte result block.
 *
 * Result layout (32 bytes):
 *   +0:  u32 status (0), u32 count before, u32 count after, u32 0
 *   +16: u32 the four messages in the order read
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

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

static unsigned char result_buf[128] __attribute__((aligned(128)));
static volatile unsigned int flag[4] __attribute__((aligned(128)));

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
    unsigned int i;

    memset(result_buf, 0xFF, sizeof(result_buf));
    flag[0] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "mboxovr";
    ret = sysSpuThreadGroupCreate(&g_group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    /* PSL1GHT's argN fields land in SPU r(3+N): arg1 is the result
     * block and arg2 the flag, the order spu/main.c takes them in. */
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "mboxovr";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    g_args.arg2 = (u64)(unsigned long)flag;
    ret = sysSpuThreadInitialize(&g_thread, g_group, 0, &g_image,
                                 &g_thrattr, &g_args);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(4);

    for (i = 1; i <= 5; i++) {
        ret = sysSpuThreadWriteMb(g_thread, i * 0x11);
        if (ret != 0) return fail(5);
    }
    __sync_synchronize();
    flag[0] = 1;
    __sync_synchronize();

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(6);

    write_tty_tagged(result_buf, 32);

    sysSpuThreadGroupDestroy(g_group);
    return 0;
}
