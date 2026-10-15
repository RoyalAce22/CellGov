/* SPU program: never started. ppu/main.c imports it as an ELF and
 * carries it flattened as a buffer that is not one.
 */

#include <sys/spu_thread.h>

int main(unsigned long long arg0, unsigned long long arg1,
         unsigned long long arg2, unsigned long long arg3)
{
    (void)arg0;
    (void)arg1;
    (void)arg2;
    (void)arg3;
    spu_thread_exit(0);
    return 0;
}
