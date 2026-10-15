/* SPU program for spu_pool_edges: exit at once, so the group the PPU
 * starts finishes.
 */

#include <sys/spu_thread.h>

int main(void)
{
    spu_thread_exit(0);
    return 0;
}
