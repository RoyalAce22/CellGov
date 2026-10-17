/* PPU probe: the priority the primary PPU thread runs at when the
 * process declares PRIO as its sys_process_param primary_prio.
 *
 * lv2_process_prio_3071's main.c differs from this copy in the PRIO line
 * only.
 *
 * Payload (big-endian u32 words):
 *   +0   PRIO, sysThreadGetId's return code, sys_ppu_thread_get_priority
 *        (48)'s return code, the priority it wrote (pre-filled with -7)
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define PRIO 3072

SYS_PROCESS_PARAM(PRIO, 0x10000)

#define SYS_PPU_THREAD_GET_PRIORITY 48

static volatile unsigned int result[4] __attribute__((aligned(128)));
static volatile int g_prio __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
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

int main(void)
{
    sys_ppu_thread_t self = 0;
    s32 ret;

    result[0] = PRIO;
    ret = sysThreadGetId(&self);
    result[1] = (unsigned int)ret;
    g_prio = -7;
    result[2] = ret == 0
        ? (unsigned int)syscall2_s32(SYS_PPU_THREAD_GET_PRIORITY, self, (u64)(unsigned long)&g_prio)
        : 0xFFFFFFFFu;
    result[3] = (unsigned int)g_prio;
    write_result();
    return 0;
}
