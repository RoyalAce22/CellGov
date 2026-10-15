/* PPU probe: which sys_config_add_service_listener (519) shapes a game
 * process may register, and the event a registered one receives.
 *
 * Every case that registers a listener removes it again (sc 520)
 * before the next case runs, except the first pad-manager listener that
 * registers: part C reads its events and removes it.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   A. setup
 *    0  sc 128 queue create rc    1  queue id
 *    2  sc 516(queue, &ch) rc     3  ch (pre-filled 0xFFFFFFFF)
 *   B. sc 519 cases: rc, listener word (pre-filled 0xFFFFFFFF), sc 520 rc
 *      or SKIP when nothing registered or the listener was kept. Case n
 *      is at slot 4 + 3n.
 *      0  service 0x11, verbosity 0, data NULL, size 0, repeating
 *      1  service 0x11, verbosity 0, {0x01}, size 1, once
 *      2  service 0x11, verbosity 0, {0x01, 0 x7}, size 8, repeating
 *      3  service 0x11, verbosity 1, {0x01}, size 1, repeating
 *      4  service 0x12, verbosity 0, {0x01}, size 1, repeating
 *      5  service 0x11, verbosity 0, {0x01, 0 x255}, size 256, repeating
 *      6  handle 0xDEADBEEF, otherwise case 2
 *      7  user service 0x8000000000000001, verbosity 0, {0x01}, size 1,
 *         repeating
 *   C. events
 *   28  sc 131 tryreceive rc after a 200 ms sleep
 *   29  events received
 *   30-33  event 0: source, data1, data2, data3
 *   34  sc 518(ch, id0, buf, 1024) rc    35  bytes written
 *   36-40  record bytes 0-39
 *   41  sc 520 of the kept listener rc, or SKIP when none was kept
 *   D. teardown
 *   42  sc 517(ch) rc             43  sc 129(queue, 1) rc
 *
 * id0 is the low word of event 0's data2, or 0 when no event arrived.
 * SKIP is 0xDEADD00D.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_QUEUE_CREATE      128
#define SYS_EVENT_QUEUE_DESTROY     129
#define SYS_EVENT_QUEUE_TRYRECEIVE  131
#define SYS_TIMER_USLEEP            141
#define SYS_CONFIG_OPEN             516
#define SYS_CONFIG_CLOSE            517
#define SYS_CONFIG_GET_EVENT        518
#define SYS_CONFIG_ADD_LISTENER     519
#define SYS_CONFIG_REMOVE_LISTENER  520

#define ONCE      0
#define REPEATING 1
#define SKIP      0xDEADD00DULL
#define BUF       1024

#define WORDS   44
#define PAYLOAD (WORDS * 8)

/* sys_event_queue_attribute_t: protocol@0 (FIFO), type@4 (PPU queue),
 * name@8. */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) = { 1, 1, 0, 0 };

struct SysEvent {
    unsigned long long source;
    unsigned long long data1;
    unsigned long long data2;
    unsigned long long data3;
};

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char buf[BUF] __attribute__((aligned(128)));
static volatile unsigned char data[256] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile int count_word __attribute__((aligned(8)));
static struct SysEvent events[4] __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

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

static u64 kept;

/* One sc 519 case. A pad-manager listener that registers while none is
 * kept becomes the kept one. */
static void add_case(unsigned n, u64 ch, u64 service, u64 verbosity, u64 ptr, u64 size,
                     u64 type)
{
    unsigned slot = 4 + 3 * n;
    u64 rc;
    w32 = 0xFFFFFFFFu;
    rc = (u64)cgov_sc(SYS_CONFIG_ADD_LISTENER, ch, service, verbosity, ptr, size, type,
                      EA(&w32), 0);
    put(slot, rc);
    put(slot + 1, w32);
    if (rc != 0) {
        put(slot + 2, SKIP);
        return;
    }
    if (kept == 0 && (service == 0x11 || service == 0x12)) {
        kept = w32;
        put(slot + 2, SKIP);
        return;
    }
    put(slot + 2, sc(SYS_CONFIG_REMOVE_LISTENER, ch, w32, 0, 0));
}

int main(void)
{
    unsigned i;
    u64 queue, ch, id0 = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof(data); i++)
        data[i] = 0;
    data[0] = 0x01;

    /* A. */
    w32 = 0;
    put(0, sc(SYS_EVENT_QUEUE_CREATE, EA(&w32), EA(queue_attr), 0, 16));
    queue = w32;
    put(1, queue);
    w32 = 0xFFFFFFFFu;
    put(2, sc(SYS_CONFIG_OPEN, queue, EA(&w32), 0, 0));
    ch = w32;
    put(3, ch);

    /* B. */
    add_case(0, ch, 0x11, 0, 0, 0, REPEATING);
    add_case(1, ch, 0x11, 0, EA(data), 1, ONCE);
    add_case(2, ch, 0x11, 0, EA(data), 8, REPEATING);
    add_case(3, ch, 0x11, 1, EA(data), 1, REPEATING);
    add_case(4, ch, 0x12, 0, EA(data), 1, REPEATING);
    add_case(5, ch, 0x11, 0, EA(data), 256, REPEATING);
    add_case(6, 0xDEADBEEFu, 0x11, 0, EA(data), 8, REPEATING);
    add_case(7, ch, 0x8000000000000001ull, 0, EA(data), 1, REPEATING);

    /* C. */
    (void)sc(SYS_TIMER_USLEEP, 200000, 0, 0, 0);
    count_word = 0;
    for (i = 0; i < 4; i++)
        events[i].source = events[i].data1 = events[i].data2 = events[i].data3 = 0;
    put(28, sc(SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), 4, EA(&count_word)));
    put(29, (u64)(unsigned int)count_word);
    put(30, events[0].source);
    put(31, events[0].data1);
    put(32, events[0].data2);
    put(33, events[0].data3);
    if (count_word > 0)
        id0 = events[0].data2 & 0xFFFFFFFFull;
    for (i = 0; i < BUF; i++)
        buf[i] = 0xEE;
    put(34, sc(SYS_CONFIG_GET_EVENT, ch, id0, EA(buf), BUF));
    i = BUF;
    while (i > 0 && buf[i - 1] == 0xEE)
        i--;
    put(35, i);
    for (i = 0; i < 5; i++)
        put(36 + i, be8(buf + 8 * i));
    put(41, kept ? sc(SYS_CONFIG_REMOVE_LISTENER, ch, kept, 0, 0) : SKIP);

    /* D. */
    put(42, sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0));
    put(43, sc(SYS_EVENT_QUEUE_DESTROY, queue, 1, 0, 0));

    write_frame();
    return 0;
}
