/* PPU probe: sys_uart_get_params (370) from a game process. It is the
 * one read-only UART call; the probe sends no other UART, PS3AV or
 * gamepad call.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   0  sc 370(buf) rc
 *   1  buf bytes 0-7 (pre-filled 0xEE)
 *   2  buf bytes 8-15 (pre-filled 0xEE)
 *   3  sc 370(NULL) rc
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_UART_GET_PARAMS 370

#define WORDS   4
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char params[16] __attribute__((aligned(16)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static u64 be8(const volatile unsigned char *p)
{
    u64 v = 0;
    unsigned i;
    for (i = 0; i < 8; i++)
        v = v << 8 | p[i];
    return v;
}

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
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof(params); i++)
        params[i] = 0xEE;

    put(0, (u64)cgov_sc(SYS_UART_GET_PARAMS, EA(params), 0, 0, 0, 0, 0, 0, 0));
    put(1, be8(params));
    put(2, be8(params + 8));
    put(3, (u64)cgov_sc(SYS_UART_GET_PARAMS, 0, 0, 0, 0, 0, 0, 0, 0));

    write_frame();
    return 0;
}
