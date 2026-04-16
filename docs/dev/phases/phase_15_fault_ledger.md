# Phase 15 Fault Ledger

| # | Fault kind  | Detail                                  | Crate          | Fix commit | New fault point       |
|---|-------------|-----------------------------------------|----------------|------------|-----------------------|
| 1 | HLE return  | cellGcmSys cluster: _cellGcmInitBody,   | cellgov_core   | (pending)  | RSX_WRITE_CHECKPOINT  |
|   |             | cellGcmGetConfiguration, GetControlReg, |                |            | at 0xC0000040         |
|   |             | GetTiledPitchSize, GetLabelAddress all  |                |            | step 14,109,359       |
|   |             | returned 0 as noop stubs. Game jumped   |                |            | (~3.6B insns, 72s)    |
|   |             | to null function pointer (CTR=0).       |                |            |                       |
|   | staging bug | StagingBuffer::drain_into panicked on   | cellgov_mem    |            |                       |
|   |             | ReservedWrite to RSX region. Pre-       |                |            |                       |
|   |             | validation checked region existence but  |                |            |                       |
|   |             | not access mode.                        |                |            |                       |
