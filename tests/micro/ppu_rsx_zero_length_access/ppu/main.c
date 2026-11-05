/* PPU probe: the forms that move no byte inside RSX local memory once
 * libgcm has brought up an RSX context.
 *
 * RSX local memory is caching-inhibited for a game process. The PPE
 * refuses the string and the VSU left and right forms there; an lvrx or
 * an stvrx at a quadword-aligned EA and an stswx with a zero count
 * name no byte. A 1 MiB-page host block and rsxInit map local memory;
 * its first 256 bytes are filled with a pattern. Each access writes a
 * progress record naming it before it runs, so a run that stops names
 * the access that stopped it; the final frame replaces the record.
 *
 * Payload (9 big-endian u64):
 *   0       DONE once every access ran, PROG in a progress record
 *   1       the step a progress record names (0 in the final frame)
 *   2       rsxInit rc (0xFFFFFFFFFFFFFFFF when not sent)
 *   3       local memory address from gcmGetConfiguration
 *   4, 5    step 1: v0 after an lvrx at local + 0x40, v0 first set to
 *           all ones (high doubleword, then low)
 *   6, 7    step 2: ld of local + 0x50 and + 0x58 after an stvrx of a
 *           vector of 0x07 bytes at local + 0x50
 *   8       step 3: ld of local + 0x60 after an stswx with an XER byte
 *           count of 0 at local + 0x60, RS holding 0x55555555
 *
 * The fill puts 0xA5A5A5A5_0000000i in u64 i of the 256 bytes.
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
#define WORDS   9
#define PAYLOAD (WORDS * 8)

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile u64 vec[2] __attribute__((aligned(16)));
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

static void progress(u64 step)
{
    result[1] = step;
    CGOV_OUT_FILE_WRITE((const void *)result, PAYLOAD);
}

int main(void)
{
    sys_mem_addr_t host = 0;
    gcmConfiguration config;
    volatile u64 *local;
    unsigned long base;
    unsigned long ea;
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
    base = (unsigned long)local;
    result[3] = base;
    for (i = 0; i < FILL_WORDS; i++)
        local[i] = FILL | i;

    progress(1);
    ea = base + 0x40;
    __asm__ volatile("vspltisb 0,-1\n\tlvrx 0,0,%0\n\tstvx 0,0,%1"
                     : : "b"(ea), "b"((unsigned long)vec) : "memory");
    result[4] = vec[0];
    result[5] = vec[1];

    progress(2);
    ea = base + 0x50;
    __asm__ volatile("vspltisb 1,7\n\tstvrx 1,0,%0" : : "b"(ea) : "memory");
    result[6] = local[10];
    result[7] = local[11];

    progress(3);
    ea = base + 0x60;
    __asm__ volatile("mtxer %1\n\tstswx %2,0,%0"
                     : : "r"(ea), "r"(0UL), "r"(0x55555555UL) : "memory", "xer");
    result[8] = local[12];

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
