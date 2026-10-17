/* PPU probe: the sys_config service records a game process receives --
 * the pad-manager service's record with the descriptor it carries, and
 * the records a user service produces when it registers and when it
 * unregisters -- plus which service ids and listener types a listener
 * with no data buffer may name.
 *
 * Every listener and service the probe registers is removed again, and
 * the queue is drained after each step, so each step reads only its own
 * events. The user service uses its own id, USER_BASE + 0x4347, not
 * libpad's.
 *
 * A pad-manager record's bytes from 55 on are zeroed before they are
 * recorded. They are the descriptor's tail, which differs from one
 * controller to the next and may identify the one attached.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   A. setup
 *    0  sc 128 queue create rc    1  queue id
 *    2  sc 516(queue, &ch) rc     3  ch (pre-filled 0xFFFFFFFF)
 *   B. pad manager: listener on 0x11, no data, repeating
 *    4  sc 519 rc                 5  listener (pre-filled 0xFFFFFFFF)
 *    6  sc 131 tryreceive rc after a 200 ms sleep
 *    7  events received
 *    8 + 13e, event e (0-1): source, data1, data2, data3; sc 518(ch,
 *       id, buf, 1024) rc, or SKIP when the event is missing; bytes
 *       written (last byte not 0xEE, plus one); record bytes 0-55
 *       (7 words, bytes 55 on zeroed). id is the low word of data2.
 *   C. after the pad-manager listener goes
 *   34  sc 520 rc
 *   35  sc 131 tryreceive rc after a 200 ms sleep   36  events received
 *   D. sc 519 cases with no data: rc, listener word, sc 520 rc or SKIP,
 *      events drained after it; case n at slot 37 + 4n
 *      0  service 0x11, once
 *      1  service 0x12, repeating
 *      2  service 0x10, repeating
 *      3  service 0x01, repeating
 *      4  service 0x11, type 2
 *      5  service USER_BASE + 1 (libpad's), repeating
 *   E. a user service of the probe's own
 *   61  sc 519 listener on USER_BASE + 0x4347, no data, repeating rc
 *   62  listener                  63  events drained after it
 *   64  sc 521(ch, USER_BASE + 0x4347, user 0, verbosity 0, NULL, 0,
 *       &service) rc              65  service (pre-filled 0xFFFFFFFF)
 *   66  sc 131 rc after a 200 ms sleep   67  events received
 *   68-71  event 0: source, data1, data2, data3
 *   72  sc 518 rc, or SKIP   73  bytes written   74-78  record bytes 0-39
 *   79  sc 522(ch, service) rc, or SKIP when nothing registered
 *   80  sc 131 rc after a 200 ms sleep   81  events received
 *   82-85  event 0: source, data1, data2, data3
 *   86  sc 518 rc, or SKIP   87  bytes written   88-92  record bytes 0-39
 *   93  sc 520 of the user listener rc, or SKIP
 *   F. teardown
 *   94  sc 517(ch) rc             95  sc 129(queue, 1) rc
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
#define SYS_CONFIG_REGISTER_SERVICE 521
#define SYS_CONFIG_UNREGISTER_SERVICE 522

#define ONCE      0
#define REPEATING 1
#define SKIP      0xDEADD00DULL
#define BUF       1024
#define EVENTS    16
#define PAD_KEPT  55
#define USER_BASE 0x8000000000000000ull
#define OWN_USER  (USER_BASE + 0x4347)

#define WORDS   96
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
static struct SysEvent events[EVENTS] __attribute__((aligned(8)));

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

/* Sleep 200 ms, then take up to EVENTS events. Returns the rc; the count
 * is in count_word. */
static u64 receive(u64 queue)
{
    unsigned i;
    (void)sc(SYS_TIMER_USLEEP, 200000, 0, 0, 0);
    count_word = 0;
    for (i = 0; i < EVENTS; i++)
        events[i].source = events[i].data1 = events[i].data2 = events[i].data3 = 0;
    return sc(SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), EVENTS, EA(&count_word));
}

/* Record event `e` at `slot`: its four words, then the sc 518 read of
 * its record, the bytes written and `words` words of the record. Bytes
 * from `kept` on are zeroed first. */
static void record(u64 ch, unsigned e, unsigned slot, unsigned words, unsigned kept)
{
    unsigned i, written;
    put(slot, events[e].source);
    put(slot + 1, events[e].data1);
    put(slot + 2, events[e].data2);
    put(slot + 3, events[e].data3);
    if ((unsigned)count_word <= e) {
        put(slot + 4, SKIP);
        return;
    }
    for (i = 0; i < BUF; i++)
        buf[i] = 0xEE;
    put(slot + 4, sc(SYS_CONFIG_GET_EVENT, ch, events[e].data2 & 0xFFFFFFFFull, EA(buf), BUF));
    written = BUF;
    while (written > 0 && buf[written - 1] == 0xEE)
        written--;
    put(slot + 5, written);
    for (i = kept; i < BUF; i++)
        buf[i] = 0;
    for (i = 0; i < words; i++)
        put(slot + 6 + i, be8(buf + 8 * i));
}

static u64 add_listener(u64 ch, u64 service, u64 type)
{
    w32 = 0xFFFFFFFFu;
    return (u64)cgov_sc(SYS_CONFIG_ADD_LISTENER, ch, service, 0, 0, 0, type, EA(&w32), 0);
}

static void add_case(unsigned n, u64 ch, u64 queue, u64 service, u64 type)
{
    unsigned slot = 37 + 4 * n;
    u64 rc = add_listener(ch, service, type);
    put(slot, rc);
    put(slot + 1, w32);
    put(slot + 2, rc == 0 ? sc(SYS_CONFIG_REMOVE_LISTENER, ch, w32, 0, 0) : SKIP);
    (void)receive(queue);
    put(slot + 3, (u64)(unsigned int)count_word);
}

int main(void)
{
    unsigned i;
    u64 queue, ch, rc, pad, user_listener, service;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    w32 = 0;
    put(0, sc(SYS_EVENT_QUEUE_CREATE, EA(&w32), EA(queue_attr), 0, 64));
    queue = w32;
    put(1, queue);
    w32 = 0xFFFFFFFFu;
    put(2, sc(SYS_CONFIG_OPEN, queue, EA(&w32), 0, 0));
    ch = w32;
    put(3, ch);

    /* B. */
    rc = add_listener(ch, 0x11, REPEATING);
    pad = w32;
    put(4, rc);
    put(5, pad);
    put(6, receive(queue));
    put(7, (u64)(unsigned int)count_word);
    record(ch, 0, 8, 7, PAD_KEPT);
    record(ch, 1, 21, 7, PAD_KEPT);

    /* C. */
    put(34, rc == 0 ? sc(SYS_CONFIG_REMOVE_LISTENER, ch, pad, 0, 0) : SKIP);
    put(35, receive(queue));
    put(36, (u64)(unsigned int)count_word);

    /* D. */
    add_case(0, ch, queue, 0x11, ONCE);
    add_case(1, ch, queue, 0x12, REPEATING);
    add_case(2, ch, queue, 0x10, REPEATING);
    add_case(3, ch, queue, 0x01, REPEATING);
    add_case(4, ch, queue, 0x11, 2);
    add_case(5, ch, queue, USER_BASE + 1, REPEATING);

    /* E. */
    rc = add_listener(ch, OWN_USER, REPEATING);
    user_listener = w32;
    put(61, rc);
    put(62, user_listener);
    (void)receive(queue);
    put(63, (u64)(unsigned int)count_word);
    w32 = 0xFFFFFFFFu;
    rc = (u64)cgov_sc(SYS_CONFIG_REGISTER_SERVICE, ch, OWN_USER, 0, 0, 0, 0, EA(&w32), 0);
    service = w32;
    put(64, rc);
    put(65, service);
    put(66, receive(queue));
    put(67, (u64)(unsigned int)count_word);
    record(ch, 0, 68, 5, BUF);
    put(79, rc == 0 ? sc(SYS_CONFIG_UNREGISTER_SERVICE, ch, service, 0, 0) : SKIP);
    put(80, receive(queue));
    put(81, (u64)(unsigned int)count_word);
    record(ch, 0, 82, 5, BUF);
    put(93, (u64)(unsigned int)user_listener != 0xFFFFFFFFull
                ? sc(SYS_CONFIG_REMOVE_LISTENER, ch, user_listener, 0, 0)
                : SKIP);

    /* F. */
    put(94, sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0));
    put(95, sc(SYS_EVENT_QUEUE_DESTROY, queue, 1, 0, 0));

    write_frame();
    return 0;
}
