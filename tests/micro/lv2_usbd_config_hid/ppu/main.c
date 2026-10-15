/* PPU probe: the read-only usbd, sys_config and hid answers a game
 * process gets.
 *
 * No pipe transfer, no LDD registration, no event wait on usbd. The
 * one usbd call that might wait (sc 541) runs on a side thread; the
 * main thread records whether it returned within the poll window.
 *
 * Payload: big-endian u64 words, one per slot below, in this order.
 *   A. hid
 *    0  sc 1 getpid return value
 *    1  sc 512(pid) rc            2  sc 512(1) rc
 *   B. usbd
 *    3  sc 530(&h1) rc            4  h1 (pre-filled 0xFFFFFFFF)
 *    5  sc 530(&h2) rc            6  h2 (pre-filled 0xFFFFFFFF)
 *    7  sc 530(NULL) rc
 *    8  sc 532(h1, list, 8) rc    9  list bytes 0-7 (pre-filled 0xEE)
 *   10  sc 536(h1, "cgovnone", 8) rc
 *   11  sc 536(0xDEADBEEF, "cgovnone", 8) rc
 *   12  sc 541() rc, or SKIP when it had not returned
 *   13  1 when sc 541 returned inside the poll window, else 0
 *   14  sc 531(h2) rc             15  sc 531(h2) again rc
 *   16  sc 531(h1) rc             17  sc 531(0xDEADBEEF) rc
 *   C. sys_config
 *   18  sc 128 queue create rc    19  queue id
 *   20  sc 516(queue, &ch) rc     21  ch (pre-filled 0xFFFFFFFF)
 *   22  sc 516(0xDEADBEEF, &x) rc
 *   23  sc 519(ch, 0x11, 0, {0x01}, 1, repeating, &lid) rc
 *   24  lid (pre-filled 0xFFFFFFFF)
 *   25  sc 131 tryreceive rc after a 200 ms sleep
 *   26  events received
 *   27-30  event 0: source, data1, data2, data3
 *   31-34  event 1: source, data1, data2, data3
 *   35  sc 518(ch, id0, buf, data3 - 1) rc, or SKIP
 *   36  bytes written (last byte not 0xEE, plus one)
 *   37  sc 518(ch, id0, buf, data3) rc, or SKIP
 *   38  bytes written
 *   39  sc 518(ch, id0, buf, 41) rc
 *   40  bytes written
 *   41  sc 518(ch, id0, buf, 1024) rc
 *   42  bytes written
 *   43-47  record bytes 0-39 from the 1024-byte read
 *   48  record bytes 40-43 in the high word (device data, first four)
 *   49  sc 518(ch, 0xDEAD, buf, 1024) rc
 *   50  sc 518(0xDEADBEEF, id0, buf, 1024) rc
 *   51  sc 520(ch, lid) rc        52  sc 520(ch, lid) again rc
 *   53  sc 517(ch) rc             54  sc 517(ch) again rc
 *   55  sc 129(queue, 1) rc
 *
 * id0 is the low word of event 0's data2, or 0 when no event arrived.
 * SKIP is 0xDEADD00D: the read was not sent because data3 was 0 or
 * above 1024.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GETPID          1
#define SYS_PPU_THREAD_EXIT         41
#define SYS_EVENT_QUEUE_CREATE      128
#define SYS_EVENT_QUEUE_DESTROY     129
#define SYS_EVENT_QUEUE_TRYRECEIVE  131
#define SYS_TIMER_USLEEP            141
#define SYS_HID_IS_ROOT             512
#define SYS_CONFIG_OPEN             516
#define SYS_CONFIG_CLOSE            517
#define SYS_CONFIG_GET_EVENT        518
#define SYS_CONFIG_ADD_LISTENER     519
#define SYS_CONFIG_REMOVE_LISTENER  520
#define SYS_USBD_INITIALIZE         530
#define SYS_USBD_FINALIZE           531
#define SYS_USBD_GET_DEVICE_LIST    532
#define SYS_USBD_UNREGISTER_LDD     536
#define SYS_USBD_DETECT_EVENT       541

#define PADMANAGER         0x11
#define LISTENER_REPEATING 1
#define SKIP               0xDEADD00DULL
#define BUF                1024

#define WORDS   56
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

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned char buf[BUF] __attribute__((aligned(128)));
static volatile unsigned char list[64] __attribute__((aligned(128)));
static volatile unsigned int w32a __attribute__((aligned(8)));
static volatile unsigned int w32b __attribute__((aligned(8)));
static volatile int count_word __attribute__((aligned(8)));
static struct SysEvent events[4] __attribute__((aligned(8)));
static const unsigned char LISTEN_DATA[4] __attribute__((aligned(8))) = { 0x01, 0, 0, 0 };
static const char PRODUCT[8] = { 'c', 'g', 'o', 'v', 'n', 'o', 'n', 'e' };
static struct cg_thread_param detect_param __attribute__((aligned(8)));
static volatile unsigned long long detect_rc;
static volatile unsigned int detect_done;

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

static void fill(volatile unsigned char *p, unsigned n)
{
    unsigned i;
    for (i = 0; i < n; i++)
        p[i] = 0xEE;
}

static u64 extent(void)
{
    unsigned n = BUF;
    while (n > 0 && buf[n - 1] == 0xEE)
        n--;
    return n;
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

static void detect_entry(void *arg)
{
    (void)arg;
    detect_rc = sc(SYS_USBD_DETECT_EVENT, 0, 0, 0, 0);
    __asm__ volatile ("lwsync" ::: "memory");
    detect_done = 1;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

/* Run sc 541 on a side thread and poll for up to 500 ms. */
static void detect_event(void)
{
    unsigned long long tid = 0;
    unsigned i;
    s32 ret;

    detect_done = 0;
    ret = cgov_ppu_thread_create(&tid,
                                 make_thread_param(&detect_param, (const void *)&detect_entry),
                                 0, 0, 1000, 0x4000, 0, 0);
    if (ret != 0) {
        put(12, (u64)(unsigned int)ret);
        put(13, 0xFF);
        return;
    }
    for (i = 0; i < 50 && !detect_done; i++)
        (void)sc(SYS_TIMER_USLEEP, 10000, 0, 0, 0);
    __asm__ volatile ("lwsync" ::: "memory");
    put(12, detect_done ? detect_rc : SKIP);
    put(13, detect_done);
}

static void get_event(unsigned rc_slot, u64 ch, u64 id, u64 size)
{
    fill(buf, BUF);
    put(rc_slot, sc(SYS_CONFIG_GET_EVENT, ch, id, EA(buf), size));
    put(rc_slot + 1, extent());
}

int main(void)
{
    unsigned i;
    u64 pid, h1, h2, queue, ch, lid, id0 = 0, data3 = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    /* A. */
    pid = sc(SYS_PROCESS_GETPID, 0, 0, 0, 0);
    put(0, pid);
    put(1, sc(SYS_HID_IS_ROOT, pid, 0, 0, 0));
    put(2, sc(SYS_HID_IS_ROOT, 1, 0, 0, 0));

    /* B. */
    w32a = 0xFFFFFFFFu;
    put(3, sc(SYS_USBD_INITIALIZE, EA(&w32a), 0, 0, 0));
    h1 = w32a;
    put(4, h1);
    w32a = 0xFFFFFFFFu;
    put(5, sc(SYS_USBD_INITIALIZE, EA(&w32a), 0, 0, 0));
    h2 = w32a;
    put(6, h2);
    put(7, sc(SYS_USBD_INITIALIZE, 0, 0, 0, 0));
    fill(list, sizeof(list));
    put(8, sc(SYS_USBD_GET_DEVICE_LIST, h1, EA(list), 8, 0));
    put(9, be8(list));
    put(10, sc(SYS_USBD_UNREGISTER_LDD, h1, EA(PRODUCT), 8, 0));
    put(11, sc(SYS_USBD_UNREGISTER_LDD, 0xDEADBEEFu, EA(PRODUCT), 8, 0));
    detect_event();
    put(14, sc(SYS_USBD_FINALIZE, h2, 0, 0, 0));
    put(15, sc(SYS_USBD_FINALIZE, h2, 0, 0, 0));
    put(16, sc(SYS_USBD_FINALIZE, h1, 0, 0, 0));
    put(17, sc(SYS_USBD_FINALIZE, 0xDEADBEEFu, 0, 0, 0));

    /* C. */
    w32a = 0;
    put(18, sc(SYS_EVENT_QUEUE_CREATE, EA(&w32a), EA(queue_attr), 0, 16));
    queue = w32a;
    put(19, queue);
    w32a = 0xFFFFFFFFu;
    put(20, sc(SYS_CONFIG_OPEN, queue, EA(&w32a), 0, 0));
    ch = w32a;
    put(21, ch);
    w32b = 0xFFFFFFFFu;
    put(22, sc(SYS_CONFIG_OPEN, 0xDEADBEEFu, EA(&w32b), 0, 0));
    w32b = 0xFFFFFFFFu;
    put(23, (u64)cgov_sc(SYS_CONFIG_ADD_LISTENER, ch, PADMANAGER, 0, EA(LISTEN_DATA), 1,
                         LISTENER_REPEATING, EA(&w32b), 0));
    lid = w32b;
    put(24, lid);

    (void)sc(SYS_TIMER_USLEEP, 200000, 0, 0, 0);
    count_word = 0;
    for (i = 0; i < 4; i++)
        events[i].source = events[i].data1 = events[i].data2 = events[i].data3 = 0;
    put(25, sc(SYS_EVENT_QUEUE_TRYRECEIVE, queue, EA(events), 4, EA(&count_word)));
    put(26, (u64)(unsigned int)count_word);
    for (i = 0; i < 2; i++) {
        put(27 + 4 * i, events[i].source);
        put(28 + 4 * i, events[i].data1);
        put(29 + 4 * i, events[i].data2);
        put(30 + 4 * i, events[i].data3);
    }
    if (count_word > 0) {
        id0 = events[0].data2 & 0xFFFFFFFFull;
        data3 = events[0].data3;
    }

    if (data3 == 0 || data3 > BUF) {
        put(35, SKIP);
        put(37, SKIP);
    } else {
        get_event(35, ch, id0, data3 - 1);
        get_event(37, ch, id0, data3);
    }
    get_event(39, ch, id0, 41);
    get_event(41, ch, id0, BUF);
    for (i = 0; i < 5; i++)
        put(43 + i, be8(buf + 8 * i));
    put(48, be8(buf + 40) & 0xFFFFFFFF00000000ull);
    put(49, sc(SYS_CONFIG_GET_EVENT, ch, 0xDEAD, EA(buf), BUF));
    put(50, sc(SYS_CONFIG_GET_EVENT, 0xDEADBEEFu, id0, EA(buf), BUF));

    put(51, sc(SYS_CONFIG_REMOVE_LISTENER, ch, lid, 0, 0));
    put(52, sc(SYS_CONFIG_REMOVE_LISTENER, ch, lid, 0, 0));
    put(53, sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0));
    put(54, sc(SYS_CONFIG_CLOSE, ch, 0, 0, 0));
    put(55, sc(SYS_EVENT_QUEUE_DESTROY, queue, 1, 0, 0));

    write_frame();
    return 0;
}
