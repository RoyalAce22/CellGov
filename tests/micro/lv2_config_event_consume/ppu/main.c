/* PPU probe: which sys_config_get_service_event (518) reads consume
 * the event they read.
 *
 * Three pad-manager listeners with no data buffer each get at least one
 * event. The first three events received are read twice each.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   A. setup
 *    0  sc 128 queue create rc    1  sc 516 rc
 *    2-4  sc 519 rc for listeners 0, 1 and 2
 *   B. events
 *    5  sc 131 tryreceive rc after a 200 ms sleep
 *    6  events received
 *    7-9  data2 of events 0, 1 and 2
 *   10-12  data3 of events 0, 1 and 2
 *   C. reads: rc, then bytes written (last byte not 0xEE, plus one);
 *      SKIP when the event did not arrive or its data3 was 0 or above
 *      1024.
 *   13  event 0, size data3          15  event 0 again, size data3
 *   17  event 1, size data3 - 1      19  event 1 again, size data3
 *   21  event 2, null destination    23  event 2 again, size data3
 *   D. teardown
 *   25  sc 520 rc for listener 0 (1 and 2 follow, then close and
 *       destroy, all ORed into this word: zero when every one is
 *       CELL_OK)
 *   26-31  zero
 *
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

#define REPEATING 1
#define SKIP      0xDEADD00DULL
#define BUF       1024

#define WORDS   32
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
static volatile unsigned int w32 __attribute__((aligned(8)));
static volatile int count_word __attribute__((aligned(8)));
static struct SysEvent events[8] __attribute__((aligned(8)));

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

static void read_event(unsigned slot, u64 ch, u64 id, u64 dst, u64 size)
{
    unsigned n;
    for (n = 0; n < BUF; n++)
        buf[n] = 0xEE;
    put(slot, sc(SYS_CONFIG_GET_EVENT, ch, id, dst, size));
    n = BUF;
    while (n > 0 && buf[n - 1] == 0xEE)
        n--;
    put(slot + 1, n);
}

static void skip(unsigned slot)
{
    put(slot, SKIP);
    put(slot + 1, SKIP);
}

int main(void)
{
    unsigned i;
    u64 queue, ch, lid[3], torn = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    w32 = 0;
    put(0, sc(SYS_EVENT_QUEUE_CREATE, EA(&w32), EA(queue_attr), 0, 16));
    queue = w32;
    w32 = 0xFFFFFFFFu;
    put(1, sc(SYS_CONFIG_OPEN, queue, EA(&w32), 0, 0));
    ch = w32;
    for (i = 0; i < 3; i++) {
        w32 = 0xFFFFFFFFu;
        put(2 + i, (u64)cgov_sc(SYS_CONFIG_ADD_LISTENER, ch, 0x11, 0, 0, 0, REPEATING,
                                EA(&w32), 0));
        lid[i] = w32;
    }

    /* B. */
    (void)sc(SYS_TIMER_USLEEP, 200000, 0, 0, 0);
    count_word = 0;
    for (i = 0; i < 8; i++)
        events[i].source = events[i].data1 = events[i].data2 = events[i].data3 = 0;
    put(5, sc(SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), 8, EA(&count_word)));
    put(6, (u64)(unsigned int)count_word);
    for (i = 0; i < 3; i++) {
        put(7 + i, events[i].data2);
        put(10 + i, events[i].data3);
    }

    /* C. */
    for (i = 0; i < 3; i++) {
        u64 id = events[i].data2 & 0xFFFFFFFFull;
        u64 d3 = events[i].data3;
        unsigned slot = 13 + 4 * i;
        if ((int)i >= count_word || d3 == 0 || d3 > BUF) {
            skip(slot);
            skip(slot + 2);
            continue;
        }
        if (i == 0)
            read_event(slot, ch, id, EA(buf), d3);
        else if (i == 1)
            read_event(slot, ch, id, EA(buf), d3 - 1);
        else
            read_event(slot, ch, id, 0, d3);
        read_event(slot + 2, ch, id, EA(buf), d3);
    }

    /* D. */
    for (i = 0; i < 3; i++)
        torn |= sc(SYS_CONFIG_REMOVE_LISTENER, ch, lid[i], 0, 0);
    torn |= sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0);
    torn |= sc(SYS_EVENT_QUEUE_DESTROY, queue, 1, 0, 0);
    put(25, torn);

    write_frame();
    return 0;
}
