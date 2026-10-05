# Console crash ledger

| Call | Status | Evidence |
| --- | --- | --- |
| `sc 801` open `O_CREAT` under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sc 811` mkdir under `/dev_flash` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| mkdir 0777 / create 0666 under `/dev_hdd0/tmp` | suspected | a run of these three plus calls that ran clean elsewhere crashed |
| `sys_semaphore_create` until refused | suspected | crashed one run, sent with every call above |
| `sys_spu_image_open` on a PPU SELF | suspected | crashed one run, sent with every call above |
