/* PPU program: an unconditional lock-line put takes another SPU's
 * reservation.
 *
 * Launches two threads of one group from the same image, both naming one
 * 128-byte lock line. Slot 0 reserves the line; slot 1 publishes over it
 * with putlluc; slot 0's putllc then fails. After join, the PPU outputs
 * the 48-byte result block.
 *
 * Result layout (48 bytes):
 *   +0:  u32 status (0), u32 getllar status, u32 putllc status, u32 0
 *   +16: u8[16] the line's first bytes after the putllc
 *   +32: u32 putlluc status, u32 0, u32 0, u32 0
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

static unsigned char result_buf[256] __attribute__((aligned(256)));
static unsigned char lock_line[128] __attribute__((aligned(128)));

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
    memset(lock_line, 0, sizeof(lock_line));

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "llucpub";
    ret = sysSpuThreadGroupCreate(&g_group, 2, 100, &g_grpattr);
    if (ret != 0) return fail(2);

    /* PSL1GHT's argN fields land in SPU r(3+N): arg0 is the slot, arg1
     * the result block and arg2 the lock line, the order spu/main.c
     * takes them in. */
    for (slot = 0; slot < 2; slot++) {
        memset(&g_thrattr[slot], 0, sizeof(g_thrattr[slot]));
        g_thrattr[slot].nsize = 8;
        g_thrattr[slot].name = "llucpub";
        memset(&g_args[slot], 0, sizeof(g_args[slot]));
        g_args[slot].arg0 = slot;
        g_args[slot].arg1 = (u64)(unsigned long)result_buf;
        g_args[slot].arg2 = (u64)(unsigned long)lock_line;
        ret = sysSpuThreadInitialize(&g_threads[slot], g_group, slot, &g_image,
                                     &g_thrattr[slot], &g_args[slot]);
        if (ret != 0) return fail(3 + slot);
    }

    ret = sysSpuThreadGroupStart(g_group);
    if (ret != 0) return fail(5);

    ret = sysSpuThreadGroupJoin(g_group, &g_cause, &g_status);
    if (ret != 0) return fail(6);

    write_tty_tagged(result_buf, 48);

    sysSpuThreadGroupDestroy(g_group);
    return 0;
}
