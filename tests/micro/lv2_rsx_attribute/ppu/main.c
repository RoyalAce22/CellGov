/* PPU probe: what sys_rsx_attribute (677) answers a game once libgcm has
 * brought up an RSX context.
 *
 * Cases, in payload order (one raw sc each, r3-r7):
 *   0  677(0, 0, 0, 0, 0)
 *   1  677(0xDEAD, 0, 0, 0, 0)
 *
 * Payload layout (24 bytes, big-endian):
 *   0x00 u64 rsxInit rc
 *   0x08 u64 r3 of case 0
 *   0x10 u64 r3 of case 1
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000
#define SYS_RSX_ATTRIBUTE 677

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[3] __attribute__((aligned(128)));
static gcmContextData *context;

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

int main(void)
{
    sys_mem_addr_t host = 0;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        result[0] = 0xA110CFA1ULL;
        write_frame(result, sizeof(result));
        return 1;
    }
    result[0] = (u64)(s64)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (result[0] != 0) {
        write_frame(result, sizeof(result));
        return 1;
    }
    result[1] = (u64)cgov_sc(SYS_RSX_ATTRIBUTE, 0, 0, 0, 0, 0, 0, 0, 0);
    result[2] = (u64)cgov_sc(SYS_RSX_ATTRIBUTE, 0xDEAD, 0, 0, 0, 0, 0, 0, 0);
    write_frame(result, sizeof(result));
    return 0;
}
