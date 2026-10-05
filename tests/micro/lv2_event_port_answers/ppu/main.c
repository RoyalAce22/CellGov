/* PPU probe: event port create types, IPC connect, send to an
 * unconnected port and to a full queue, destroy of a connected port,
 * and a tryreceive batch larger than the queue holds.
 *
 * Every step records its return code, in this order (word index):
 *   A. port types
 *     0-3   sc 134(&port, type, 0) for type 0, 1, 2, 3; a port that is
 *           created is destroyed at once (sc 135)
 *   B. IPC connect
 *     4     sc 128 an IPC queue (key K, size 4)
 *     5     sc 134 an IPC port (type 3)
 *     6     sc 140(port, K)
 *     7     sc 140(port, K) again, on the connected port
 *     8     sc 137 disconnect
 *     9     sc 140(port, 0)
 *     10    sc 140(port, K + 1), a key that names no queue
 *     11    sc 134 a local port (type 1)
 *     12    sc 140(local port, K)
 *     13    sc 135 the local port
 *     14    sc 140(port, K), connecting again
 *     15    sc 135 the connected IPC port
 *     16    sc 137 disconnect
 *     17    sc 135 the IPC port
 *     18    sc 129 the IPC queue
 *   C. send
 *     19    sc 134 a local port
 *     20    sc 138 send on it while unconnected
 *     21    sc 128 a local queue of size 1
 *     22    sc 136 connect the port to it
 *     23    sc 138 send (data1 = 0x11)
 *     24    sc 138 send again, the queue full
 *   D. tryreceive
 *     25    sc 128 a local queue of size 4
 *     26    sc 134 a local port; 27 sc 136 connect it
 *     28,29 sc 138 two sends (data1 = 0x21, 0x22)
 *     30    sc 131(queue, events, 3, &n)
 *     31    n (prefilled with 0xFFFFFFFF)
 *     32,33 data1 of events 0 and 1, low words
 *     34    data1 of event 2, low word (prefilled with 0xFFFFFFFF)
 *   E. cleanup
 *     35    sc 137 + sc 135 the C port; 36 sc 129 the C queue
 *     37    sc 137 + sc 135 the D port; 38 sc 129 the D queue
 *     39    zero
 *
 * Payload: 40 big-endian u32 words, 160 bytes.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_QUEUE_CREATE      128
#define SYS_EVENT_QUEUE_DESTROY     129
#define SYS_EVENT_QUEUE_TRYRECEIVE  131
#define SYS_EVENT_PORT_CREATE       134
#define SYS_EVENT_PORT_DESTROY      135
#define SYS_EVENT_PORT_CONNECT_LOCAL 136
#define SYS_EVENT_PORT_DISCONNECT   137
#define SYS_EVENT_PORT_SEND         138
#define SYS_EVENT_PORT_CONNECT_IPC  140

#define PORT_LOCAL 1
#define PORT_IPC   3

/* An IPC key this probe owns: no system service uses it. */
#define IPC_KEY 0x0000C0DE14760001ULL

/* sys_event_queue_attribute_t: protocol@0 (FIFO), type@4 (PPU queue),
 * name@8. */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) = { 1, 1, 0, 0 };

struct SysEvent {
    unsigned long long source;
    unsigned long long data1;
    unsigned long long data2;
    unsigned long long data3;
};

static unsigned int result[40] __attribute__((aligned(128)));
static unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int count_word __attribute__((aligned(8)));
static struct SysEvent events[3] __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

#define EA(p) ((unsigned long)(const void *)(p))

static unsigned int port_create(unsigned int type, unsigned int at)
{
    id_word = 0;
    result[at] = (unsigned int)syscall3_s32(SYS_EVENT_PORT_CREATE, EA(&id_word), type, 0);
    return id_word;
}

static unsigned int queue_create(unsigned long long key, unsigned int size, unsigned int at)
{
    id_word = 0;
    result[at] = (unsigned int)syscall4_s32(SYS_EVENT_QUEUE_CREATE, EA(&id_word),
                                            EA(queue_attr), key, size);
    return id_word;
}

static unsigned int sc1(unsigned int num, unsigned int a)
{
    return (unsigned int)syscall1_s32(num, a);
}

int main(void)
{
    unsigned int t, port, ipc_port, local, queue, cport, cqueue, dport, dqueue;
    unsigned int i;

    memset(result, 0, sizeof(result));

    /* A. */
    for (t = 0; t < 4; t++) {
        port = port_create(t, t);
        if (result[t] == 0)
            (void)sc1(SYS_EVENT_PORT_DESTROY, port);
    }

    /* B. */
    queue = queue_create(IPC_KEY, 4, 4);
    ipc_port = port_create(PORT_IPC, 5);
    result[6] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, ipc_port, IPC_KEY);
    result[7] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, ipc_port, IPC_KEY);
    result[8] = sc1(SYS_EVENT_PORT_DISCONNECT, ipc_port);
    result[9] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, ipc_port, 0);
    result[10] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, ipc_port, IPC_KEY + 1);
    local = port_create(PORT_LOCAL, 11);
    result[12] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, local, IPC_KEY);
    result[13] = sc1(SYS_EVENT_PORT_DESTROY, local);
    result[14] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_IPC, ipc_port, IPC_KEY);
    result[15] = sc1(SYS_EVENT_PORT_DESTROY, ipc_port);
    result[16] = sc1(SYS_EVENT_PORT_DISCONNECT, ipc_port);
    result[17] = sc1(SYS_EVENT_PORT_DESTROY, ipc_port);
    result[18] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, queue, 0);

    /* C. */
    cport = port_create(PORT_LOCAL, 19);
    result[20] = (unsigned int)syscall4_s32(SYS_EVENT_PORT_SEND, cport, 0x10, 0, 0);
    cqueue = queue_create(0, 1, 21);
    result[22] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_LOCAL, cport, cqueue);
    result[23] = (unsigned int)syscall4_s32(SYS_EVENT_PORT_SEND, cport, 0x11, 0, 0);
    result[24] = (unsigned int)syscall4_s32(SYS_EVENT_PORT_SEND, cport, 0x12, 0, 0);

    /* D. */
    dqueue = queue_create(0, 4, 25);
    dport = port_create(PORT_LOCAL, 26);
    result[27] = (unsigned int)syscall2_s32(SYS_EVENT_PORT_CONNECT_LOCAL, dport, dqueue);
    result[28] = (unsigned int)syscall4_s32(SYS_EVENT_PORT_SEND, dport, 0x21, 0, 0);
    result[29] = (unsigned int)syscall4_s32(SYS_EVENT_PORT_SEND, dport, 0x22, 0, 0);
    for (i = 0; i < 3; i++) {
        events[i].source = 0xFFFFFFFFFFFFFFFFULL;
        events[i].data1 = 0xFFFFFFFFFFFFFFFFULL;
        events[i].data2 = 0xFFFFFFFFFFFFFFFFULL;
        events[i].data3 = 0xFFFFFFFFFFFFFFFFULL;
    }
    count_word = 0xFFFFFFFFu;
    result[30] = (unsigned int)syscall4_s32(SYS_EVENT_QUEUE_TRYRECEIVE, dqueue, EA(events), 3,
                                            EA(&count_word));
    result[31] = count_word;
    result[32] = (unsigned int)events[0].data1;
    result[33] = (unsigned int)events[1].data1;
    result[34] = (unsigned int)events[2].data1;

    /* E. */
    (void)sc1(SYS_EVENT_PORT_DISCONNECT, cport);
    result[35] = sc1(SYS_EVENT_PORT_DESTROY, cport);
    result[36] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, cqueue, 0);
    (void)sc1(SYS_EVENT_PORT_DISCONNECT, dport);
    result[37] = sc1(SYS_EVENT_PORT_DESTROY, dport);
    result[38] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, dqueue, 0);

    write_result();
    return 0;
}
