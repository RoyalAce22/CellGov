# Console crash ledger

| Call | Status | Evidence |
| --- | --- | --- |
| `sc 801` open `O_CREAT` under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sc 811` mkdir under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| mkdir 0777 / create 0666 under `/dev_hdd0/tmp` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sys_semaphore_create` until refused | suspected | crashed one run, sent with every call above |
| `sys_spu_image_open` on a PPU SELF | suspected | crashed one run, sent with every call above |
| raw SPU list put whose element is over 16 KB or misaligned (an MFC class 0 error with the class 0 mask at 0) | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| `sysSpuRawGetIntStat` of class 1 | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| `sysSpuRawDestroy` of a raw SPU whose MFC stopped on an error, then a second `sysSpuRawCreate` | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| `_sys_prx_get_module_list` (494) with the pad word, `count` and every byte from 0x18 holding `0xAA` | confirmed | a module-list run whose only calls no clean run had sent were two of these (flags 2 and 0, size 0x20) never wrote its result and the console rebooted; flags 0 to 4 and sizes 0x20 and 0x28 with those words zero ran clean |
| `_sys_prx_get_module_list` (494) with flags 2 and a null `pInfo` | confirmed | a module-list run whose only call no clean run had sent was this one stopped answering until a power cycle; the same run without it was clean |
