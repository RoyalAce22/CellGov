/* PPU probe: what the RSX context calls hand a game through libgcm, and
 * what the raw RSX calls answer on paths no libgcm caller takes.
 *
 * Part 1 brings up an RSX context with rsxInit (libgcm's cellGcmInit)
 * over a 1 MiB host area, then records what libgcm learned from the
 * kernel: the configuration, the DMA control block, the label, notify
 * and report addresses, the IO map cap, and the label block's contents.
 * The driver-info block is guessed at one MiB below the label block. A
 * write of that range to a scratch file under /dev_hdd0/tmp tells
 * whether it is readable: an unmapped range answers EFAULT there
 * instead of faulting the probe. Only a readable range is copied.
 *
 * Part 2 sends raw calls, each recording its r3:
 *   669 sys_rsx_memory_free(0xDEAD)
 *   671 sys_rsx_context_free(0xDEAD)
 *   668 sys_rsx_memory_allocate with size 0
 *   672 iomap(0x55555555, io 0x100000, a second host area, 1 MiB)
 *   672 iomap(0x55555555, io 0x200000, ea 0xC0000000, 1 MiB)
 *   672 iomap(0x55555555, io 0x10000000, the second host area, 1 MiB)
 *   674 attribute(0x55555555, 0x102 FLIP_BUFFER, head 0, 0x8000000F)
 *
 * Payload layout (9208 bytes, big-endian):
 *   0x000 u32 rsxInit rc
 *   0x004 u32 localAddress   0x008 u32 ioAddress
 *   0x00C u32 localSize      0x010 u32 ioSize
 *   0x014 u32 memoryFreq     0x018 u32 coreFreq
 *   0x01C u32 DMA control block address (gcmGetControlRegister)
 *   0x020 u32 label 0 address
 *   0x024 u32 notify entry 0 address
 *   0x028 u32 report entry 0 address
 *   0x02C u32 max IO map size
 *   0x030 u32 driver-info guess address (label 0 - 0x100000)
 *   0x034 u32 its readability write rc (0 = readable)
 *   0x038 u32 bytes that write took
 *   0x03C u32 second host area address
 *   0x040 u64 r3 of 669         0x048 u64 r3 of 671
 *   0x050 u64 r3 of 668 size 0  0x058 u32 its handle   0x05C pad
 *   0x060 u64 its address
 *   0x068 u64 r3 of the valid iomap
 *   0x070 u64 r3 of the 0xC0000000 iomap
 *   0x078 u64 r3 of the io-past-256-MiB iomap
 *   0x080 u64 r3 of the flip
 *   0x088 zero to 0x100
 *   0x100 0x1000 bytes from label 0
 *   0x1100 0x12F8 bytes from the driver-info guess (zero unless readable)
 */

#include <string.h>
#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE   (1024 * 1024)
#define CMD_SIZE    0x10000
#define LABEL_BYTES 0x1000
#define DRIVER_INFO 0x12F8
#define PAYLOAD     (0x1100 + DRIVER_INFO)
#define CONTEXT_ID  0x55555555ULL

#define SYS_RSX_MEMORY_ALLOCATE   668
#define SYS_RSX_MEMORY_FREE       669
#define SYS_RSX_CONTEXT_FREE      671
#define SYS_RSX_CONTEXT_IOMAP     672
#define SYS_RSX_CONTEXT_ATTRIBUTE 674
#define SYS_FS_OPEN   801
#define SYS_FS_WRITE  803
#define SYS_FS_CLOSE  804
#define SYS_FS_UNLINK 814
#define O_WRONLY  0x1
#define O_CREAT   0x40
#define O_TRUNC   0x200
#define MODE_FILE 0x180

#define EA(p) ((u64)(unsigned long)(p))

static const char SCRATCH[] = "/dev_hdd0/tmp/cgov_rsx_scratch";
static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned char result[PAYLOAD] __attribute__((aligned(128)));
static gcmContextData *context;
static volatile u32 handle_word __attribute__((aligned(8)));
static volatile u64 addr_word;
static volatile u32 fd_word __attribute__((aligned(8)));
static volatile u64 nwrite_word;

static void write_frame(const void *data, unsigned int len)
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

static void put_u32(unsigned int off, u32 v)
{
    unsigned int i;
    for (i = 0; i < 4; i++)
        result[off + i] = (unsigned char)(v >> (24 - 8 * i));
}

static void put_u64(unsigned int off, u64 v)
{
    unsigned int i;
    for (i = 0; i < 8; i++)
        result[off + i] = (unsigned char)(v >> (56 - 8 * i));
}

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f, u64 g)
{
    return (u64)cgov_sc(num, a, b, c, d, e, f, g, 0);
}

/* Whether `len` bytes at `at` are readable: the kernel copies them into
 * a scratch file and answers EFAULT when it cannot. */
static u32 readable(u32 at, u32 len)
{
    u64 rc;

    rc = sc(SYS_FS_OPEN, EA(SCRATCH), O_WRONLY | O_CREAT | O_TRUNC, EA(&fd_word),
            MODE_FILE, 0, 0, 0);
    if (rc != 0)
        return (u32)rc;
    nwrite_word = 0;
    rc = sc(SYS_FS_WRITE, fd_word, at, len, EA(&nwrite_word), 0, 0, 0);
    put_u32(0x038, (u32)nwrite_word);
    (void)sc(SYS_FS_CLOSE, fd_word, 0, 0, 0, 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(SCRATCH), 0, 0, 0, 0, 0, 0);
    return (u32)rc;
}

int main(void)
{
    sys_mem_addr_t host = 0, host2 = 0;
    gcmConfiguration config;
    u32 label0, guess, ok;
    s32 rc;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0
        || sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host2) != 0) {
        put_u32(0x000, 0xA110CFA1u);
        write_frame(result, sizeof(result));
        return 1;
    }
    put_u32(0x03C, (u32)host2);
    rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    put_u32(0x000, (u32)rc);
    if (rc != 0) {
        write_frame(result, sizeof(result));
        return 1;
    }

    memset(&config, 0, sizeof(config));
    gcmGetConfiguration(&config);
    /* The two addresses are 32-bit PRX pointers: copy their words. */
    memcpy(&result[0x004], &config.localAddress, 4);
    memcpy(&result[0x008], &config.ioAddress, 4);
    put_u32(0x00C, config.localSize);
    put_u32(0x010, config.ioSize);
    put_u32(0x014, config.memoryFreq);
    put_u32(0x018, config.coreFreq);
    put_u32(0x01C, (u32)(unsigned long)gcmGetControlRegister());
    label0 = (u32)(unsigned long)gcmGetLabelAddress(0);
    put_u32(0x020, label0);
    put_u32(0x024, (u32)(unsigned long)gcmGetNotifyDataAddress(0));
    put_u32(0x028, (u32)(unsigned long)gcmGetReportDataAddress(0));
    put_u32(0x02C, gcmGetMaxIoMapSize());
    memcpy(&result[0x100], (const void *)(unsigned long)label0, LABEL_BYTES);

    guess = label0 - 0x100000;
    put_u32(0x030, guess);
    ok = readable(guess, DRIVER_INFO);
    put_u32(0x034, ok);
    if (ok == 0)
        memcpy(&result[0x1100], (const void *)(unsigned long)guess, DRIVER_INFO);

    put_u64(0x040, sc(SYS_RSX_MEMORY_FREE, 0xDEAD, 0, 0, 0, 0, 0, 0));
    put_u64(0x048, sc(SYS_RSX_CONTEXT_FREE, 0xDEAD, 0, 0, 0, 0, 0, 0));
    handle_word = 0xCCCCCCCCu;
    addr_word = 0xCCCCCCCCCCCCCCCCULL;
    put_u64(0x050, sc(SYS_RSX_MEMORY_ALLOCATE, EA(&handle_word), EA(&addr_word), 0, 0,
                      0, 0, 0));
    put_u32(0x058, handle_word);
    put_u64(0x060, addr_word);
    put_u64(0x068, sc(SYS_RSX_CONTEXT_IOMAP, CONTEXT_ID, 0x100000, host2, HOST_SIZE, 0,
                      0, 0));
    put_u64(0x070, sc(SYS_RSX_CONTEXT_IOMAP, CONTEXT_ID, 0x200000, 0xC0000000ULL,
                      HOST_SIZE, 0, 0, 0));
    put_u64(0x078, sc(SYS_RSX_CONTEXT_IOMAP, CONTEXT_ID, 0x10000000ULL, host2,
                      HOST_SIZE, 0, 0, 0));
    put_u64(0x080, sc(SYS_RSX_CONTEXT_ATTRIBUTE, CONTEXT_ID, 0x102, 0, 0x8000000FULL, 0,
                      0, 0));

    write_frame(result, sizeof(result));
    return 0;
}
