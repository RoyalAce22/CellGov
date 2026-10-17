# Console crash ledger

| Call | Status | Evidence |
| --- | --- | --- |
| `sc 801` open `O_CREAT` under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sc 811` mkdir under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| mkdir 0777 / create 0666 under `/dev_hdd0/tmp` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sys_semaphore_create` until refused | suspected | crashed one run, sent with every call above |
| `sys_spu_image_open` on a PPU SELF | suspected | crashed one run, sent with every call above |
| raw SPU `MFC_WrTagUpdate` written with the reserved value 3 | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| raw SPU `MFC_WrListStallAck` with no list stalled | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| raw SPU `MFC_Cmd` written alone after a command, its parameters not rewritten | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| raw SPU list put whose element is over 16 KB or misaligned (an MFC class 0 error with the class 0 mask at 0) | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| `sysSpuRawGetIntStat` of class 1 | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
| `sysSpuRawDestroy` of a raw SPU whose MFC stopped on an error, then a second `sysSpuRawCreate` | suspected | a raw SPU MFC edge run sent these, wrote its whole result, destroyed its three raw SPUs and then stopped answering until a power cycle; two of its raw SPUs ended with the MFC stopped on a class 0 DMA alignment error |
