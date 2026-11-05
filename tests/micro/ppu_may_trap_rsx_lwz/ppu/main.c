/* PPU probe: what a misaligned lwz does inside RSX local memory once
 * libgcm has brought up an RSX context.
 *
 * A 1 MiB-page host block and rsxInit map local memory; its first 256
 * bytes are filled with a pattern. One lwz runs at local + 0x81. It may
 * end or stop the process (the PPE refuses a misaligned load on
 * caching-inhibited storage), so a progress record is written to the
 * result file before it runs; the final frame replaces it.
 *
 * Payload (6 big-endian u64):
 *   0       DONE once the lwz ran, PROG in the progress record
 *   1       1 in the progress record (0 in the final frame)
 *   2       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   3       local memory address from gcmGetConfiguration
 *   4       lwz EA
 *   5       lwz result (SKIP until the lwz completes)
 *
 * The fill puts 0xA5A5A5A5_0000000i in u64 i of the 256 bytes, so the
 * word at local + 0x81 is 0xA5A5A500.
 * SKIP is 0xDEADD00D.
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000

#define DONE 0x444F4E45ULL
#define PROG 0x50524F47ULL
#define SKIP 0xDEADD00DULL
#define FILL 0xA5A5A5A500000000ULL

#define FILL_WORDS 32
#define WORDS   6
#define PAYLOAD (WORDS * 8)

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static gcmContextData *context;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
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
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    volatile u64 *local;
    unsigned long ea;
    unsigned int loaded;
    unsigned i;

    result[0] = PROG;
    result[1] = 0;
    result[2] = ~0ULL;
    for (i = 3; i < WORDS; i++)
        result[i] = SKIP;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        write_frame();
        return 1;
    }
    result[2] = (u64)(s64)rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (result[2] != 0) {
        write_frame();
        return 1;
    }
    gcmGetConfiguration(&config);
    local = (volatile u64 *)config.localAddress;
    result[3] = (u64)(unsigned long)local;
    for (i = 0; i < FILL_WORDS; i++)
        local[i] = FILL | i;

    ea = (unsigned long)local + 0x81;
    result[4] = ea;
    result[1] = 1;
    CGOV_OUT_FILE_WRITE((const void *)result, PAYLOAD);
    __asm__ volatile("lwz %0,0(%1)" : "=r"(loaded) : "b"(ea) : "memory");
    result[5] = loaded;

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
