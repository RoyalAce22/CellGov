/* PPU probe: the buffer sizes sys_config_get_service_event (518)
 * accepts for a pad-manager event, and the bytes it writes.
 *
 * The listener carries no data buffer: the lv2_config_listener_shapes
 * capture shows a game process gets ENOMEM for a pad-manager listener
 * with one.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   A. setup
 *    0  sc 128 queue create rc    1  queue id
 *    2  sc 516(queue, &ch) rc     3  ch (pre-filled 0xFFFFFFFF)
 *    4  sc 519(ch, 0x11, 0, NULL, 0, repeating, &lid) rc
 *    5  lid (pre-filled 0xFFFFFFFF)
 *   B. events
 *    6  sc 131 tryreceive rc after a 200 ms sleep
 *    7  events received
 *    8-11   event 0: source, data1, data2, data3
 *   12-15   event 1: source, data1, data2, data3
 *   C. sc 518(ch, id0, buf, size): rc, then bytes written (last byte
 *      not 0xEE, plus one). Case n is at slot 16 + 2n; SKIP when no
 *      event arrived or data3 was 0 or above 1024.
 *      0  size data3 - 1    1  size data3      2  size 41
 *      3  size 40           4  size 24         5  size 0
 *      6  size data3, a null destination
 *   30  record bytes 40-47 from case 1 (the first eight data bytes)
 *   31  record bytes 48-55 from case 1
 *   32  sc 518(ch, id1, buf, 1024) rc    33  bytes written
 *   D. teardown
 *   34  sc 520(ch, lid) rc        35  sc 517(ch) rc
 *   36  sc 129(queue, 1) rc
 *   37  zero
 *
 * id0 and id1 are the low words of events 0 and 1's data2, or 0 when
 * the event did not arrive. SKIP is 0xDEADD00D.
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

#define WORDS   38
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

int main(void)
{
    unsigned i;
    u64 queue, ch, lid, id0 = 0, id1 = 0, data3 = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    w32 = 0;
    put(0, sc(SYS_EVENT_QUEUE_CREATE, EA(&w32), EA(queue_attr), 0, 16));
    queue = w32;
    put(1, queue);
    w32 = 0xFFFFFFFFu;
    put(2, sc(SYS_CONFIG_OPEN, queue, EA(&w32), 0, 0));
    ch = w32;
    put(3, ch);
    w32 = 0xFFFFFFFFu;
    put(4, (u64)cgov_sc(SYS_CONFIG_ADD_LISTENER, ch, 0x11, 0, 0, 0, REPEATING, EA(&w32), 0));
    lid = w32;
    put(5, lid);

    /* B. */
    (void)sc(SYS_TIMER_USLEEP, 200000, 0, 0, 0);
    count_word = 0;
    for (i = 0; i < 4; i++)
        events[i].source = events[i].data1 = events[i].data2 = events[i].data3 = 0;
    put(6, sc(SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), 4, EA(&count_word)));
    put(7, (u64)(unsigned int)count_word);
    for (i = 0; i < 2; i++) {
        put(8 + 4 * i, events[i].source);
        put(9 + 4 * i, events[i].data1);
        put(10 + 4 * i, events[i].data2);
        put(11 + 4 * i, events[i].data3);
    }
    if (count_word > 0) {
        id0 = events[0].data2 & 0xFFFFFFFFull;
        data3 = events[0].data3;
    }
    if (count_word > 1)
        id1 = events[1].data2 & 0xFFFFFFFFull;

    /* C. */
    if (count_word == 0 || data3 == 0 || data3 > BUF) {
        for (i = 0; i < 7; i++) {
            put(16 + 2 * i, SKIP);
            put(17 + 2 * i, SKIP);
        }
    } else {
        read_event(16, ch, id0, EA(buf), data3 - 1);
        read_event(18, ch, id0, EA(buf), data3);
        put(30, be8(buf + 40));
        put(31, be8(buf + 48));
        read_event(20, ch, id0, EA(buf), 41);
        read_event(22, ch, id0, EA(buf), 40);
        read_event(24, ch, id0, EA(buf), 24);
        read_event(26, ch, id0, EA(buf), 0);
        read_event(28, ch, id0, 0, data3);
    }
    read_event(32, ch, id1, EA(buf), BUF);

    /* D. */
    put(34, sc(SYS_CONFIG_REMOVE_LISTENER, ch, lid, 0, 0));
    put(35, sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0));
    put(36, sc(SYS_EVENT_QUEUE_DESTROY, queue, 1, 0, 0));

    write_frame();
    return 0;
}
