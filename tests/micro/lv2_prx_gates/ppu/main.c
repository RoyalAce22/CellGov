/* PPU probe: what the PRX start and stop gates and syscall 462 answer
 * for an unknown id. Nothing is loaded or created.
 *
 * Section A -- the start (481) and stop (482) gates, one raw sc each,
 * r3 = id, r4 = 0, r5 = pOpt:
 *   A0  482(0xDEADBEEF, NULL)   unknown id and a null option
 *   A1  481(0xDEADBEEF, NULL)
 *   A2  482(0, NULL)            id 0 and a null option
 *   A3  481(0, NULL)
 *   A4  482(0xDEADBEEF, opt)    unknown id, a readable cmd-1 option
 *   A5  481(0xDEADBEEF, opt)
 *
 * Section B -- syscall 462 with every argument zero.
 *
 * Payload layout (56 bytes, big-endian):
 *   0x000  A0-A5: u64 r3 each
 *   0x030  B: u64 r3
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PRX_START_MODULE 481
#define SYS_PRX_STOP_MODULE 482
#define SYS_UNOWNED_462 462
#define BAD_ID 0xDEADBEEFULL
#define PAYLOAD 0x38

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned char result[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 opt[5] __attribute__((aligned(16)));

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

static void put_u64(unsigned int off, u64 v)
{
    unsigned int i;
    for (i = 0; i < 8; i++)
        result[off + i] = (unsigned char)(v >> (56 - 8 * i));
}

static u64 sc3(u64 num, u64 a, u64 b, u64 c)
{
    return (u64)cgov_sc(num, a, b, c, 0, 0, 0, 0, 0);
}

static void seed_opt(void)
{
    opt[0] = 0x28;
    opt[1] = 1;
    opt[2] = 0x1111111111111111ULL;
    opt[3] = 0;
    opt[4] = 0x2222222222222222ULL;
}

int main(void)
{
    const u64 opt_ptr = (u64)(unsigned long)opt;

    put_u64(0x00, sc3(SYS_PRX_STOP_MODULE, BAD_ID, 0, 0));
    put_u64(0x08, sc3(SYS_PRX_START_MODULE, BAD_ID, 0, 0));
    put_u64(0x10, sc3(SYS_PRX_STOP_MODULE, 0, 0, 0));
    put_u64(0x18, sc3(SYS_PRX_START_MODULE, 0, 0, 0));
    seed_opt();
    put_u64(0x20, sc3(SYS_PRX_STOP_MODULE, BAD_ID, 0, opt_ptr));
    seed_opt();
    put_u64(0x28, sc3(SYS_PRX_START_MODULE, BAD_ID, 0, opt_ptr));

    put_u64(0x30, (u64)cgov_sc(SYS_UNOWNED_462, 0, 0, 0, 0, 0, 0, 0, 0));

    write_frame(result, sizeof(result));
    return 0;
}
