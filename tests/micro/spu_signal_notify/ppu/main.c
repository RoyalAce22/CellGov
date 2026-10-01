/* PPU program: sys_spu_thread_write_snr in both signal modes.
 *
 * Initializes one SPU thread and, before starting its group, sets its
 * configuration to SPU_SIGNAL2_OR (register 1 overwrites, register 2
 * ORs) and reads it back. After the start it writes 0x10 then 0x20 to
 * register 1 and 0x01 then 0x02 to register 2, tries a register number
 * of 2 and an unknown thread, then sets a flag the SPU polls. The SPU
 * reads both counts and both registers. After join the PPU outputs the
 * 48-byte result block.
 *
 * Result layout (48 bytes):
 *   +0:  u32 status (0), u32 configuration read back,
 *        u32 result of register number 2, u32 result of an unknown thread
 *   +16: u32 count 1, u32 count 2, u32 register 1, u32 register 2
 *   +32: u32 count 1 after, u32 count 2 after,
 *        u32 result of a configuration of 4, u32 result of reading an
 *        unknown thread's configuration
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

static unsigned int result_buf[32] __attribute__((aligned(128)));
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
    u64 cfg = 0xFFFFFFFF;
    unsigned int bad_number, bad_thread;

    memset(result_buf, 0xFF, sizeof(result_buf));
    flag[0] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "sigsnr";
    ret = sysSpuThreadGroupCreate(&g_group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    /* PSL1GHT's argN fields land in SPU r(3+N): arg1 is the result
     * block and arg2 the flag, the order spu/main.c takes them in. */
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "sigsnr";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    g_args.arg2 = (u64)(unsigned long)flag;
    ret = sysSpuThreadInitialize(&g_thread, g_group, 0, &g_image,
                                 &g_thrattr, &g_args);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadSetConfiguration(g_thread, SPU_SIGNAL1_OVERWRITE | SPU_SIGNAL2_OR);
    if (ret != 0) return fail(4);
    ret = sysSpuThreadGetConfiguration(g_thread, &cfg);
    if (ret != 0) return fail(5);

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(6);

    if (sysSpuThreadWriteSignal(g_thread, 0, 0x10) != 0) return fail(7);
    if (sysSpuThreadWriteSignal(g_thread, 0, 0x20) != 0) return fail(8);
    if (sysSpuThreadWriteSignal(g_thread, 1, 0x01) != 0) return fail(9);
    if (sysSpuThreadWriteSignal(g_thread, 1, 0x02) != 0) return fail(10);
    bad_number = (unsigned int)sysSpuThreadWriteSignal(g_thread, 2, 0x40);
    bad_thread = (unsigned int)sysSpuThreadWriteSignal(0xFFFFFFFF, 0, 0x40);

    __sync_synchronize();
    flag[0] = 1;
    __sync_synchronize();

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(11);

    result_buf[1] = (unsigned int)cfg;
    result_buf[2] = bad_number;
    result_buf[3] = bad_thread;
    result_buf[10] = (unsigned int)sysSpuThreadSetConfiguration(g_thread, 4);
    result_buf[11] = (unsigned int)sysSpuThreadGetConfiguration(0xFFFFFFFF, &cfg);
    write_tty_tagged(result_buf, 48);

    sysSpuThreadGroupDestroy(g_group);
    return 0;
}
