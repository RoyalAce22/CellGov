/* PPU program: SPU-to-SPU transfer through the SPU thread window.
 *
 * Launches two threads of one group from the same image. Slot 0 signals
 * slot 1 through slot 1's alias; slot 1 waits on the signal, gets a
 * buffer from slot 0's local store through slot 0's alias, and writes a
 * 32-byte result block. After join, the PPU outputs the block.
 *
 * Result layout (32 bytes):
 *   +0:  u32 status (0), u32 signal word, u32 receiver slot (1), u32 0
 *   +16: u8[16] the bytes the receiver read from the sender's local store
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

static unsigned char result_buf[256] __attribute__((aligned(256)));

static sysSpuImage g_image;
static sys_spu_group_t g_group;
static sys_spu_thread_t g_threads[2];
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr[2];
static sysSpuThreadArgument g_args[2];
static unsigned int g_cause;
static unsigned int g_status;

int main(void)
{
    int ret;
    unsigned int slot;

    memset(result_buf, 0xFF, sizeof(result_buf));

    ret = sysSpuImageOpen(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "lsalias";
    ret = sysSpuThreadGroupCreate(&g_group, 2, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    /* PSL1GHT's argN fields land in SPU r(3+N): arg0 is the slot and
     * arg1 the result block, the order spu/main.c takes them in. */
    for (slot = 0; slot < 2; slot++) {
        memset(&g_thrattr[slot], 0, sizeof(g_thrattr[slot]));
        g_thrattr[slot].nsize = 8;
        g_thrattr[slot].name = "lsalias";
        memset(&g_args[slot], 0, sizeof(g_args[slot]));
        g_args[slot].arg0 = slot;
        g_args[slot].arg1 = (u64)(unsigned long)result_buf;
        ret = sysSpuThreadInitialize(&g_threads[slot], g_group, slot, &g_image,
                                     &g_thrattr[slot], &g_args[slot]);
        if (ret != 0) return fail(3 + slot);
    }

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(5);

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(6);

    write_tty_tagged(result_buf, 32);

    sysSpuThreadGroupDestroy(g_group);
    return 0;
}
