/* PPU probe: the timer sleeps, the time queries, timer create and destroy,
 * and sys_tty_write on a bad buffer and a null length pointer.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   0  sc 141 usleep(1000) rc        1  timebase ticks it took
 *   2  sc 141 usleep(0) rc           3  timebase ticks it took
 *   4  sc 142 sleep(1) rc            5  timebase ticks it took
 *   6  sc 142 sleep(0) rc            7  timebase ticks it took
 *   8  sc 144(&tz, &dst) rc          9  tz word (8 bytes, pre-filled 0xEE)
 *  10  dst word (8 bytes, pre-filled 0xEE)
 *  11  sc 144(NULL, &dst) rc        12  dst word
 *  13  sc 144(&tz, NULL) rc         14  tz word
 *  15  sc 145(&sec, &nsec) rc       16  sec (8 bytes)    17  nsec (8 bytes)
 *  18  sc 145(NULL, &nsec) rc       19  nsec word
 *  20  sc 147 return value          21  the timebase at that call
 *  22  sc 71 destroy(0xDEADBEEF) rc
 *  23  sc 70 create rc              24  the id word (pre-filled 0xFFFFFFFF)
 *  25  sc 71 destroy(id) rc         26  sc 71 destroy(id) again rc
 *  27  sc 403(0, 0xDEAD0000, 16, &n) rc       28  n (pre-filled 0xEEEEEEEE)
 *  29  sc 403(0, buf, 4, NULL) rc
 *  30  sc 403(0, buf, 0, &n) rc                31  n (pre-filled 0xEEEEEEEE)
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_TIMER_CREATE                70
#define SYS_TIMER_DESTROY               71
#define SYS_TIMER_USLEEP                141
#define SYS_TIMER_SLEEP                 142
#define SYS_TIME_GET_TIMEZONE           144
#define SYS_TIME_GET_CURRENT_TIME       145
#define SYS_TIME_GET_TIMEBASE_FREQUENCY 147
#define SYS_TTY_WRITE                   403

#define WORDS   32
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char w8a[8] __attribute__((aligned(8)));
static volatile unsigned char w8b[8] __attribute__((aligned(8)));
static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int n_word __attribute__((aligned(8)));
static const char TTY_TEXT[4] = { 'p', 'r', 'b', '\n' };

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static u64 raw8(const volatile unsigned char *p)
{
    u64 v = 0;
    unsigned i;
    for (i = 0; i < 8; i++)
        v = v << 8 | p[i];
    return v;
}

static void fill(volatile unsigned char *p)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        p[i] = 0xEE;
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

static void timed(unsigned slot, u64 num, u64 arg)
{
    u64 before = timebase();
    u64 rc = sc(num, arg, 0, 0, 0);
    put(slot, rc);
    put(slot + 1, timebase() - before);
}

int main(void)
{
    unsigned i;
    u64 id;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    timed(0, SYS_TIMER_USLEEP, 1000);
    timed(2, SYS_TIMER_USLEEP, 0);
    timed(4, SYS_TIMER_SLEEP, 1);
    timed(6, SYS_TIMER_SLEEP, 0);

    fill(w8a);
    fill(w8b);
    put(8, sc(SYS_TIME_GET_TIMEZONE, EA(w8a), EA(w8b), 0, 0));
    put(9, raw8(w8a));
    put(10, raw8(w8b));
    fill(w8b);
    put(11, sc(SYS_TIME_GET_TIMEZONE, 0, EA(w8b), 0, 0));
    put(12, raw8(w8b));
    fill(w8a);
    put(13, sc(SYS_TIME_GET_TIMEZONE, EA(w8a), 0, 0, 0));
    put(14, raw8(w8a));

    fill(w8a);
    fill(w8b);
    put(15, sc(SYS_TIME_GET_CURRENT_TIME, EA(w8a), EA(w8b), 0, 0));
    put(16, raw8(w8a));
    put(17, raw8(w8b));
    fill(w8b);
    put(18, sc(SYS_TIME_GET_CURRENT_TIME, 0, EA(w8b), 0, 0));
    put(19, raw8(w8b));

    put(20, sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0));
    put(21, timebase());

    put(22, sc(SYS_TIMER_DESTROY, 0xDEADBEEFu, 0, 0, 0));
    id_word = 0xFFFFFFFFu;
    put(23, sc(SYS_TIMER_CREATE, EA(&id_word), 0, 0, 0));
    id = id_word;
    put(24, id);
    put(25, sc(SYS_TIMER_DESTROY, id, 0, 0, 0));
    put(26, sc(SYS_TIMER_DESTROY, id, 0, 0, 0));

    n_word = 0xEEEEEEEEu;
    put(27, sc(SYS_TTY_WRITE, 0, 0xDEAD0000u, 16, EA(&n_word)));
    put(28, n_word);
    put(29, sc(SYS_TTY_WRITE, 0, EA(TTY_TEXT), 4, 0));
    n_word = 0xEEEEEEEEu;
    put(30, sc(SYS_TTY_WRITE, 0, EA(TTY_TEXT), 0, EA(&n_word)));
    put(31, n_word);

    write_frame();
    return 0;
}
