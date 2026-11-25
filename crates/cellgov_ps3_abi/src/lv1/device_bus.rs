//! The facts of the device bus hypercalls: the devices, their MMIO and
//! DMA regions, the I/O segments and the IOMMU entries.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! # `open_device`
//!
//! The handler [OFW-4.93 lv1 a:0x305A38] does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`OPEN_DEVICE_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x305AC8-0x305ACC].
//! 2. It calls the open function [OFW-4.93 lv1 a:0x2375B0] with the
//!    caller's partition, r3, r4 and r5 [OFW-4.93 lv1 a:0x305A4C-0x305A68]
//!    [OFW-4.93 lv1 a:0x305AD0-0x305AEC].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x305AF0]
//!    [OFW-4.93 lv1 a:0x305B58]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The open function does these steps:
//! 1. It refuses a bus id other than 1, 2, 3 and 4 with
//!    [`OPEN_DEVICE_NO_BUS_R3`] [OFW-4.93 lv1 a:0x2375C0-0x2375E8]
//!    [OFW-4.93 lv1 a:0x237714-0x237754].
//! 2. It looks up the device [OFW-4.93 lv1 a:0x2375EC-0x2375F4]. When the
//!    lookup finds no device, it returns [`OPEN_DEVICE_NO_DEVICE_R3`]
//!    [OFW-4.93 lv1 a:0x237600] [OFW-4.93 lv1 a:0x237780-0x237790].
//! 3. For bus 1, it takes a path of its own for each of the devices 0, 1
//!    and 2 [OFW-4.93 lv1 a:0x237604-0x237608]
//!    [OFW-4.93 lv1 a:0x237658-0x23765C] [OFW-4.93 lv1 a:0x237758-0x237764].
//!
//! # Device 0 of bus 1
//!
//! The lookup for bus 1 [OFW-4.93 lv1 a:0x236594-0x2365AC]
//! [OFW-4.93 lv1 a:0x236648-0x236658] reads device 0 from slot 0 of the
//! system bus object [OFW-4.93 lv1 a:0x235D88-0x235DA0]
//! [OFW-4.93 lv1 a:0x235DC8-0x235DE8] [OFW-4.93 lv1 a:0x22B838-0x22B844].
//! lv1's init fills that slot [OFW-4.93 lv1 a:0x3A9924-0x3A9948]. It then
//! gives the device two MMIO regions [OFW-4.93 lv1 a:0x3A993C-0x3A9968],
//! which are the regions that the 4.93 kernel maps after its open
//! [OFW-4.93 lv2_kernel a:0x800000000009370C-0x8000000000093734].
//!
//! The device's constructor sets its method table
//! [OFW-4.93 lv1 a:0x22DDC0-0x22DDD4] [OFW-4.93 lv1 a:0x353480-0x353484].
//! The table's open entry [OFW-4.93 lv1 a:0x35C890-0x35C894]
//! [OFW-4.93 lv1 a:0x32FA48-0x32FA4C] calls the table's entry at +16
//! [OFW-4.93 lv1 a:0x35C888-0x35C88C] [OFW-4.93 lv1 a:0x32D4F8-0x32D4FC],
//! and sign-extends its r3 [OFW-4.93 lv1 a:0x226058-0x22608C]. That entry
//! holds the device's lock and does these steps:
//! - When the device's open flag at +108 is set, it returns `LV1_BUSY`
//!   [OFW-4.93 lv1 a:0x2280A0-0x2280AC].
//! - Otherwise it keeps the caller's partition at +112, sets the open flag
//!   and returns 0 [OFW-4.93 lv1 a:0x2280B0-0x2280BC].
//!
//! No step reads r5 [OFW-4.93 lv1 a:0x228078-0x2280D4].
//!
//! The open function calls that entry with the bus 1 lock held
//! [OFW-4.93 lv1 a:0x237660-0x2376BC]. It returns a nonzero r3 of the
//! entry zero-extended [OFW-4.93 lv1 a:0x2376C0-0x2376C8]. After a 0, it runs the
//! hook [OFW-4.93 lv1 a:0x23E2F8] [OFW-4.93 lv1 a:0x237794-0x2377B0]. The
//! hook gives the global object at +16 a 288-byte object of its own and
//! stores to a register block [OFW-4.93 lv1 a:0x23E174-0x23E1F8]
//! [OFW-4.93 lv1 a:0x242860-0x2428F4]. When the hook fails, the function
//! closes the device again and returns the hook's status zero-extended
//! [OFW-4.93 lv1 a:0x2377B4-0x2377D8]. Otherwise it returns 0
//! [OFW-4.93 lv1 a:0x2377A8].
//!
//! The hook fails only on state of lv1's own:
//! - the object at +16 is absent [OFW-4.93 lv1 a:0x23E2F8-0x23E31C];
//! - an earlier hook already gave that object its state 2
//!   [OFW-4.93 lv1 a:0x23E33C-0x23E348];
//! - the 288-byte allocation fails [OFW-4.93 lv1 a:0x23E19C-0x23E1B8];
//! - the object's first byte is not 0 or 1
//!   [OFW-4.93 lv1 a:0x23E194-0x23E1A8] [OFW-4.93 lv1 a:0x242890-0x2428A4].
//!
//! No input of the call selects these cases. Another lv1 path runs the
//! same hook with the global object's field at +8
//! [OFW-4.93 lv1 a:0x247164-0x247170].
//!
//! The 4.93 kernel's call opens device 0 of bus 1 with r5 = 0
//! [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718]. Its
//! caller does not test the result
//! [OFW-4.93 lv2_kernel a:0x80000000000936F0-0x800000000009370C].
//!
//! # Device 1 of bus 1
//!
//! The lookup reads device 1 from slot 3 of the system bus object
//! [OFW-4.93 lv1 a:0x235DA4-0x235DAC] [OFW-4.93 lv1 a:0x235DC8-0x235DE8].
//! lv1's init fills that slot with the constructor of slot 0
//! [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A70] [OFW-4.93 lv1 a:0x3A9758-0x3A9780].
//! The device's method table is therefore the table of device 0.
//!
//! The open function does not call the device's open entry for device 1
//! [OFW-4.93 lv1 a:0x237758-0x23775C]. It does these steps instead
//! [OFW-4.93 lv1 a:0x2377DC-0x2377F0]:
//! 1. It gets the controller object from slot 0 of an array
//!    [OFW-4.93 lv1 a:0x23D0F8-0x23D104]. lv1's init makes the
//!    controller for slot 3 and stores it there
//!    [OFW-4.93 lv1 a:0x23DA60-0x23DAC4] [OFW-4.93 lv1 a:0x23DBC8-0x23DBD0].
//!    A later init step keeps the device of slot 3 in the controller
//!    [OFW-4.93 lv1 a:0x3AB040-0x3AB068].
//! 2. It calls the controller's open with the caller's partition.
//! 3. It returns that open's r3 zero-extended.
//!
//! The controller's open holds the controller's lock and does these steps:
//! - It calls the device's entry at +16 with the partition
//!   [OFW-4.93 lv1 a:0x23DC54-0x23DC74]. That entry is the open entry of
//!   device 0 [OFW-4.93 lv1 a:0x2280A0-0x2280BC].
//! - When the entry fails, it returns the entry's r3 sign-extended
//!   [OFW-4.93 lv1 a:0x23DC78-0x23DCBC].
//! - Otherwise it asks the device's entry at +48 whether the partition
//!   holds the device open [OFW-4.93 lv1 a:0x23DCC0-0x23DCE0]. That entry
//!   returns 1 when the open flag is set and the stored partition is the
//!   caller's [OFW-4.93 lv1 a:0x228120-0x228144]. A 0 gives -5
//!   [OFW-4.93 lv1 a:0x23DCE4-0x23DCF4].
//! - After a 1, it sets the controller's state word to 2 and returns 0
//!   [OFW-4.93 lv1 a:0x23DD14-0x23DD28]. When the state word was 3, it
//!   first writes the controller's registers
//!   [OFW-4.93 lv1 a:0x23DD2C-0x23DD94].
//!
//! The entry at +48 runs right after the entry at +16 opened the device
//! for the same partition, so it returns 1. No step reads r5, and no hook
//! runs after the open.
//!
//! # Device 2 of bus 1
//!
//! Device 2 takes the path of device 1 with the next slot and the next
//! controller:
//! - the lookup reads it from slot 4 [OFW-4.93 lv1 a:0x235DB0-0x235DB8];
//! - lv1's init fills that slot with the same constructor
//!   [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AC4];
//! - the open function calls the open of the controller in slot 1 of the
//!   array [OFW-4.93 lv1 a:0x237760-0x23777C], which lv1's init makes for
//!   slot 4 [OFW-4.93 lv1 a:0x23DB28-0x23DB54]
//!   [OFW-4.93 lv1 a:0x23DBD4-0x23DBDC].
//!
//! # `allocate_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306A6C] keeps r3 to r7
//! [OFW-4.93 lv1 a:0x306A80] [OFW-4.93 lv1 a:0x306AA0-0x306AAC] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_REGION_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306B0C-0x306B10]
//!    [OFW-4.93 lv1 a:0x306BE8-0x306BF8].
//! 2. It refuses flags above [`DMA_REGION_FLAGS_MAX`] with
//!    [`DMA_REGION_FLAGS_RANGE_R3`] [OFW-4.93 lv1 a:0x306B14-0x306B18]
//!    [OFW-4.93 lv1 a:0x306BFC-0x306C0C].
//! 3. It zeroes the return slot of r4, HSPRG0 - 0x6A38, and calls the
//!    allocate function [OFW-4.93 lv1 a:0x236F88] with the caller's
//!    partition, r3 to r7 and the address of that slot
//!    [OFW-4.93 lv1 a:0x306B1C-0x306B54].
//! 4. It returns that function's r3 [OFW-4.93 lv1 a:0x306B58]
//!    [OFW-4.93 lv1 a:0x306BC0]. The vector's entry zeroes the other
//!    slots [OFW-4.93 lv1 a:0x0C08], so r5 to r10 return 0.
//!
//! The allocate function writes the slot only on success
//! [OFW-4.93 lv1 a:0x23707C-0x237088], so a failure returns r4 = 0. For
//! device 0 of bus 1 it does these steps. The first failure is the
//! answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236FE0-0x236FE8], which
//!    [`network`](super::network) describes. Bus 0 gets
//!    [`DMA_REGION_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`DMA_REGION_NOT_OPEN_R3`].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236FEC-0x237004]. For bus 1 it
//!    is the lookup of the access check, so it finds device 0.
//! 3. Bus 1 takes a path of its own for devices 1 and 2
//!    [OFW-4.93 lv1 a:0x237008-0x23700C]
//!    [OFW-4.93 lv1 a:0x237108-0x237118]. Device 0 continues.
//! 4. Flags other than 0 get [`DMA_REGION_FLAGS_R3`]
//!    [OFW-4.93 lv1 a:0x237010-0x237024].
//! 5. The device's allocate function [OFW-4.93 lv1 a:0x2267FC], with the
//!    device, the partition, r5, r6 and two outputs
//!    [OFW-4.93 lv1 a:0x237058-0x237070]: the address and a register
//!    mark. On a nonzero r3 the allocate function zero-extends it and
//!    sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x2370B8-0x2370E4]. On 0 it stores the address in
//!    the slot of r4 and returns 0 [OFW-4.93 lv1 a:0x23707C-0x237088].
//!
//! The device's allocate function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x226820-0x226844] and does these steps:
//! 1. A size of 0 gets [`DMA_REGION_SIZE_R3`]
//!    [OFW-4.93 lv1 a:0x226848-0x22684C] [OFW-4.93 lv1 a:0x226928-0x226930].
//! 2. A page shift other than those of [`DMA_REGION_PAGE_SHIFTS`] gets
//!    [`DMA_REGION_PAGE_SHIFT_R3`] [OFW-4.93 lv1 a:0x226850-0x226878]
//!    [OFW-4.93 lv1 a:0x226964-0x22696C].
//! 3. A size that is not a multiple of the page gets
//!    [`DMA_REGION_SIZE_R3`] [OFW-4.93 lv1 a:0x226880-0x22688C].
//! 4. It clears the mark and calls the device's entry at +152 with the
//!    device, the size, the page shift and the partition
//!    [OFW-4.93 lv1 a:0x226890-0x2268B0]. A nonzero r3 of the entry is
//!    the answer [OFW-4.93 lv1 a:0x2268B4-0x2268D0].
//! 5. Otherwise it gives the caller +48 of the region that the entry
//!    made [OFW-4.93 lv1 a:0x2268EC-0x2268F4].
//!
//! Device 0's entry at +152 [OFW-4.93 lv1 a:0x35C910-0x35C914]
//! [OFW-4.93 lv1 a:0x32D300-0x32D304] does these steps:
//! 1. It calls the device's entry at +88 [OFW-4.93 lv1 a:0x226BF0-0x226C1C],
//!    which picks the bus address; see below. A nonzero r3 is the answer,
//!    sign-extended [OFW-4.93 lv1 a:0x226C20-0x226C58].
//! 2. It allocates an 80-byte region [OFW-4.93 lv1 a:0x226C5C-0x226C70].
//!    It fills the region [OFW-4.93 lv1 a:0x225CAC-0x225CDC] with the
//!    partition at +8, the address at +16 and +48, the size at +56 and
//!    the page shift at +64.
//! 3. It links the region into the device's list at +96 and returns 0
//!    [OFW-4.93 lv1 a:0x226C8C-0x226D0C].
//!
//! When the allocation of the region fails, the entry returns
//! `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x226D88-0x226D94]. No
//! input of the call selects that case.
//!
//! Device 0's entry at +88 [OFW-4.93 lv1 a:0x35C8D0-0x35C8D4]
//! [OFW-4.93 lv1 a:0x32E0E0-0x32E0E4] does these steps:
//! 1. When +120 of the device holds -1, it returns -17
//!    [OFW-4.93 lv1 a:0x22D990-0x22D9B0]. lv1's init fills +120
//!    [OFW-4.93 lv1 a:0x22DDF8-0x22DE08], and the capture
//!    [`DMA_REGION_CAPTURE`] reads 2 there.
//! 2. It calls the pool function [OFW-4.93 lv1 a:0x22D3A4] with the
//!    device's backend at +136 and index at +128
//!    [OFW-4.93 lv1 a:0x22D9B4-0x22D9BC]. lv1's init sets them to the
//!    system bus object and 0 [OFW-4.93 lv1 a:0x3A9924-0x3A9938]
//!    [OFW-4.93 lv1 a:0x3A9768-0x3A9780] [OFW-4.93 lv1 a:0x22DDD8-0x22DDDC].
//!    So the pool is slot 0 of that object, the device itself.
//! 3. The pool function rounds the size up to a power of two; see
//!    [`dma_region_rounded_size`]. A rounded size above
//!    [`DMA_REGION_SIZE_MAX`] gets -17 [OFW-4.93 lv1 a:0x22D3FC-0x22D42C]
//!    [OFW-4.93 lv1 a:0x22D4B8-0x22D4D0] [OFW-4.93 lv1 a:0x22D56C-0x22D580].
//!    The allocate function zero-extends it with no mark, so the answer
//!    is [`DMA_REGION_TOO_LARGE_R3`].
//! 4. It searches the pool for a free range of that size
//!    [OFW-4.93 lv1 a:0x22D434-0x22D440] [OFW-4.93 lv1 a:0x22CEF8]. The
//!    search walks lists that lv1's own allocations change, under a
//!    global lock [OFW-4.93 lv1 a:0x22AC9C-0x22AD44].
//! 5. For index 0 it subtracts the sum of the pool's words at +168 and
//!    +176 from the found address, and adds 0x80000000
//!    [OFW-4.93 lv1 a:0x22D584-0x22D598] [OFW-4.93 lv1 a:0x22D6B8-0x22D6F0].
//!
//! The bus address therefore depends on the pool's state, which
//! CellGov does not model. [`GAME_OS_DMA_REGIONS`] gives the address
//! that the console's lv1 gave the 4.93 kernel's call.
//!
//! # The 4.93 kernel's call of `allocate_device_dma_region`
//!
//! The kernel's wrapper sends the call with these inputs
//! [OFW-4.93 lv2_kernel a:0x8000000000095848-0x800000000009585C]:
//! - r3 and r4: the bus id at +144 and the device id at +152 of its
//!   device record [OFW-4.93 lv2_kernel a:0x80000000000AA488-0x80000000000AA4A0];
//! - r5: the size;
//! - r6: the base-2 logarithm of the page size, 12, 16, 20 or 24
//!   [OFW-4.93 lv2_kernel a:0x8000000000095794-0x80000000000957FC];
//! - r7: 0.
//!
//! When r3 is 0, the wrapper stores r4 as the region's address
//! [OFW-4.93 lv2_kernel a:0x8000000000095860-0x8000000000095880].
//!
//! # `map_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306770] keeps r3 to r8
//! [OFW-4.93 lv1 a:0x306784-0x3067B8] and does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_MAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306818-0x30681C]
//!    [OFW-4.93 lv1 a:0x306904-0x306914].
//! 2. It refuses flags that set a bit outside [`DMA_MAP_FLAGS_MASK`] with
//!    [`DMA_MAP_FLAGS_R3`] [OFW-4.93 lv1 a:0x306820-0x30683C].
//! 3. It calls the map function [OFW-4.93 lv1 a:0x236D1C] with the
//!    caller's partition and r3 to r8 [OFW-4.93 lv1 a:0x3068D0-0x3068F8].
//! 4. It returns that function's r3 [OFW-4.93 lv1 a:0x3068FC]
//!    [OFW-4.93 lv1 a:0x3068A4]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The map function does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236D74], which `allocate_device_dma_region` also
//!    calls. Bus 0 gets [`DMA_MAP_NO_BUS_R3`]. A partition that did not
//!    open the device gets [`DMA_MAP_NOT_OPEN_R3`]. The function
//!    zero-extends the status and sets bit (mark + 31) for a mark other
//!    than 0 [OFW-4.93 lv1 a:0x236D88] [OFW-4.93 lv1 a:0x236E1C-0x236E38].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236D90]. For bus 1 it finds
//!    device 0. The function takes no path of its own for devices 1 and
//!    2 of bus 1.
//! 3. The device's map function [OFW-4.93 lv1 a:0x227218], with the
//!    device, the partition, the bus address, the logical partition
//!    address, the length, the flags and a mark
//!    [OFW-4.93 lv1 a:0x236D94-0x236DB8]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x236DBC-0x236DE4].
//!
//! The device's map function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x227268-0x22728C] and does these steps:
//! 1. It finds the region that holds the bus address with the device's
//!    entry at +200 [OFW-4.93 lv1 a:0x227290-0x2272B8]
//!    [OFW-4.93 lv1 a:0x35C940-0x35C944] [OFW-4.93 lv1 a:0x32D450-0x32D454].
//!    That entry walks the device's region list at +96. It checks no
//!    partition [OFW-4.93 lv1 a:0x22799C-0x2279D0]. When no region holds
//!    the address, the answer is [`DMA_MAP_NO_REGION_R3`]
//!    [OFW-4.93 lv1 a:0x227444-0x22744C].
//! 2. A length of 0 or above the region's size gets
//!    [`DMA_MAP_LENGTH_R3`] [OFW-4.93 lv1 a:0x2272BC-0x2272CC]
//!    [OFW-4.93 lv1 a:0x2273E8-0x2273F0].
//! 3. The range check [OFW-4.93 lv1 a:0x2257C4-0x225848]; see
//!    [`dma_map_range_free`]. A range that fails it gets
//!    [`DMA_MAP_BUSY_R3`] [OFW-4.93 lv1 a:0x2272D0-0x2272E4]
//!    [OFW-4.93 lv1 a:0x2273F8-0x227400].
//! 4. The page holds 2^(page shift) bytes of the region
//!    [OFW-4.93 lv1 a:0x2272E8-0x227328]. The checks of
//!    [`dma_map_alignment_r3`] run next.
//! 5. It clears the mark and calls the device's entry at +168
//!    [OFW-4.93 lv1 a:0x22735C-0x227398] [OFW-4.93 lv1 a:0x35C920-0x35C924]
//!    [OFW-4.93 lv1 a:0x32D480-0x32D484]. It returns that entry's r3
//!    sign-extended [OFW-4.93 lv1 a:0x2273B0].
//!
//! Device 0's entry at +168 [OFW-4.93 lv1 a:0x227B34] does these steps:
//! 1. It calls the device's entry at +120, which gives the I/O address of
//!    the bus address [OFW-4.93 lv1 a:0x227BDC-0x227C10]
//!    [OFW-4.93 lv1 a:0x35C8F0-0x35C8F4] [OFW-4.93 lv1 a:0x32E008-0x32E00C].
//!    That entry returns -17 in these cases
//!    [OFW-4.93 lv1 a:0x22D718-0x22D760]:
//!    - the 32-bit word at +160 of the device is 16;
//!    - the bus address is outside [`DMA_MAP_IO_BUS_ADDRESSES`].
//!
//!    The capture [`DMA_REGION_CAPTURE`] reads 2 in that word.
//! 2. It calls the device's entry at +136
//!    [OFW-4.93 lv1 a:0x227C50-0x227C98] [OFW-4.93 lv1 a:0x35C900-0x35C904]
//!    [OFW-4.93 lv1 a:0x32D360-0x32D364]. That entry writes the IOPTE of
//!    each page with an lv1 function [OFW-4.93 lv1 a:0x2D7E9C], and
//!    returns -17 when one write fails [OFW-4.93 lv1 a:0x227124-0x2271E4].
//!    The writes read lv1's IOMMU state, which CellGov does not model.
//! 3. It makes the mapping with the mapping function
//!    [OFW-4.93 lv1 a:0x225E18] [OFW-4.93 lv1 a:0x227C9C]. That function
//!    repeats the range check [OFW-4.93 lv1 a:0x225F7C-0x226048], so only
//!    its 96-byte allocation can fail [OFW-4.93 lv1 a:0x225E60-0x225E74].
//!    On that failure, the entry undoes the writes and returns
//!    `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x227D08-0x227D2C]. No
//!    input of the call selects that case.
//! 4. It stores the logical partition address at +80 of the mapping and
//!    returns 0 [OFW-4.93 lv1 a:0x227CC4-0x227CD0].
//!
//! The mapping function fills the mapping with the partition, the offset
//! of the bus address in the region, the logical partition address and
//! the length. It does not keep the flags
//! [OFW-4.93 lv1 a:0x225E78-0x225EB4]. It links the mapping into the
//! region's list at +72 [OFW-4.93 lv1 a:0x225EB8-0x225F50].
//!
//! # The 4.93 kernel's call of `map_device_dma_region`
//!
//! The kernel's caller gives its wrapper 0 as the flags word
//! [OFW-4.93 lv2_kernel a:0x80000000000954a4-0x80000000000954d4]. So the
//! wrapper sets the IOPTE flags to 0xE000000000000000
//! [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc]. For
//! each page of its buffer, the wrapper sends the call with these inputs
//! [OFW-4.93 lv2_kernel a:0x800000000007d678-0x800000000007d6b4]:
//! - r3 and r4: the bus id and the device id;
//! - r5: the logical partition address of the page;
//! - r6: the bus address of the page;
//! - r7: the page size;
//! - r8: the flags.
//!
//! A status of -9 or -2 makes the wrapper unmap the pages that it mapped
//! [OFW-4.93 lv2_kernel a:0x800000000007d6c4-0x800000000007d73c]. The
//! capture [`DMA_MAP_CAPTURE`] gives lv1's record of the call; see
//! [`GAME_OS_DMA_MAPPINGS`].

use std::ops::RangeInclusive;

use super::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_WRONG_STATE};

/// The handler of hypercall 170 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenDeviceHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `open_device` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 170 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const OPEN_DEVICE_HANDLERS: &[OpenDeviceHandler] = &[OpenDeviceHandler {
    firmware: "4.93",
    handler: 0x305A38,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn open_device_handler(firmware: &str) -> Option<&'static OpenDeviceHandler> {
    OPEN_DEVICE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest bus id that the handler gives the open function.
pub const OPEN_DEVICE_BUS_MAX: u64 = 4;

/// The r3 of `open_device` for a bus id above [`OPEN_DEVICE_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x305B78-0x305B84].
pub const OPEN_DEVICE_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `open_device` for bus id 0: `LV1_NO_ENTRY` (-6), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x237724-0x237750].
pub const OPEN_DEVICE_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `open_device` when the lookup finds no device:
/// `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x237780-0x23778C].
pub const OPEN_DEVICE_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `open_device` for a device that is open: `LV1_BUSY` (-9),
/// zero-extended.
///
/// The device's open entry returns -9 [OFW-4.93 lv1 a:0x2280A4]. The open
/// function clears the high word on each path:
/// - for device 0 of bus 1 [OFW-4.93 lv1 a:0x2376C4];
/// - for device 1 of bus 1, after the controller's open returns the -9
///   [OFW-4.93 lv1 a:0x23DC9C] [OFW-4.93 lv1 a:0x2377EC];
/// - for device 2 of bus 1, on the same terms [OFW-4.93 lv1 a:0x237778].
pub const OPEN_DEVICE_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The bus of the devices whose open path CellGov read.
pub const OPEN_DEVICE_READ_BUS: u64 = 1;

/// The device ids, on [`OPEN_DEVICE_READ_BUS`], whose open path CellGov
/// read, ascending.
pub const OPEN_DEVICE_READ_DEVICES: &[u64] = &[0, 1, 2];

/// Whether CellGov read lv1's open path for device `device` of bus `bus`.
pub fn open_device_path_read(bus: u64, device: u64) -> bool {
    bus == OPEN_DEVICE_READ_BUS && OPEN_DEVICE_READ_DEVICES.contains(&device)
}

/// The r3 of the handler's bus checks for the bus id `bus`, or `None`
/// when `bus` passes them:
/// - a bus id above [`OPEN_DEVICE_BUS_MAX`] gets
///   [`OPEN_DEVICE_BUS_RANGE_R3`];
/// - bus id 0 gets [`OPEN_DEVICE_NO_BUS_R3`].
pub const fn open_device_bus_r3(bus: u64) -> Option<u64> {
    if bus > OPEN_DEVICE_BUS_MAX {
        Some(OPEN_DEVICE_BUS_RANGE_R3)
    } else if bus == 0 {
        Some(OPEN_DEVICE_NO_BUS_R3)
    } else {
        None
    }
}

/// The handler of hypercall 174 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRegionHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `allocate_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 174 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_REGION_HANDLERS: &[DmaRegionHandler] = &[DmaRegionHandler {
    firmware: "4.93",
    handler: 0x306A6C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_region_handler(firmware: &str) -> Option<&'static DmaRegionHandler> {
    DMA_REGION_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `allocate_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306BE8-0x306BF4].
///
/// Bit 32 marks r3, bit 33 r4, bit 34 r5, bit 35 r6 and bit 36 r7.
pub const DMA_REGION_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The largest flags value that the handler passes, unsigned
/// [OFW-4.93 lv1 a:0x306B14-0x306B18].
pub const DMA_REGION_FLAGS_MAX: u64 = 3;

/// The r3 for flags above [`DMA_REGION_FLAGS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x306BFC-0x306C08].
pub const DMA_REGION_FLAGS_RANGE_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x2370B8-0x2370C4].
pub const DMA_REGION_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for device 0 of bus 1 with flags other than 0:
/// `LV1_UNSUPPORTED_PARAMETER_VALUE` (-15), with r7 marked
/// [OFW-4.93 lv1 a:0x237018-0x237024].
pub const DMA_REGION_FLAGS_R3: u64 = 0x0000_0010_FFFF_FFF1;

/// The r3 for a size of 0 or a size that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x226928-0x226930] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_SIZE_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a page shift that is not in [`DMA_REGION_PAGE_SHIFTS`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x226964-0x22696C] [OFW-4.93 lv1 a:0x2370B8-0x2370E4].
pub const DMA_REGION_PAGE_SHIFT_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The page shifts that the device's allocate function accepts: pages of
/// 4 KB, 64 KB, 1 MB and 16 MB [OFW-4.93 lv1 a:0x226850-0x22687C].
pub const DMA_REGION_PAGE_SHIFTS: [u64; 4] = [12, 16, 20, 24];

/// The largest rounded size that the pool function accepts
/// [OFW-4.93 lv1 a:0x22D420-0x22D42C] [OFW-4.93 lv1 a:0x22D4C0-0x22D4CC].
pub const DMA_REGION_SIZE_MAX: u64 = 1 << 32;

/// The r3 for a rounded size above [`DMA_REGION_SIZE_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), zero-extended
/// [OFW-4.93 lv1 a:0x22D4D0] [OFW-4.93 lv1 a:0x2370B8-0x2370C4].
pub const DMA_REGION_TOO_LARGE_R3: u64 = zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);

/// The size that the pool function searches for, from the nonzero size
/// `size`.
///
/// A power of two stays as it is [OFW-4.93 lv1 a:0x22D404-0x22D41C].
/// Another size becomes twice its highest set bit
/// [OFW-4.93 lv1 a:0x22D4B8-0x22D4BC] [OFW-4.93 lv1 a:0x22D56C-0x22D570].
/// lv1 doubles at 64 bits, so a size above 2^63 that is not a power of
/// two becomes 0.
pub const fn dma_region_rounded_size(size: u64) -> u64 {
    if size == 0 {
        return 0;
    }
    let top = 1u64 << (63 - size.leading_zeros());
    if top == size {
        size
    } else {
        top.wrapping_shl(1)
    }
}

/// The r3 of the size and page shift checks for `size` and `page_shift`,
/// or `None` when they pass.
///
/// The checks run in this order:
/// 1. a size of 0 gets [`DMA_REGION_SIZE_R3`];
/// 2. a page shift that is not in [`DMA_REGION_PAGE_SHIFTS`] gets
///    [`DMA_REGION_PAGE_SHIFT_R3`];
/// 3. a size that is not a multiple of the page gets
///    [`DMA_REGION_SIZE_R3`];
/// 4. a rounded size above [`DMA_REGION_SIZE_MAX`] gets
///    [`DMA_REGION_TOO_LARGE_R3`].
///
/// lv1 compares each value at 64 bits.
pub const fn dma_region_shape_r3(size: u64, page_shift: u64) -> Option<u64> {
    if size == 0 {
        return Some(DMA_REGION_SIZE_R3);
    }
    let shifts = DMA_REGION_PAGE_SHIFTS;
    if page_shift != shifts[0]
        && page_shift != shifts[1]
        && page_shift != shifts[2]
        && page_shift != shifts[3]
    {
        return Some(DMA_REGION_PAGE_SHIFT_R3);
    }
    if !size.is_multiple_of(1u64 << page_shift) {
        return Some(DMA_REGION_SIZE_R3);
    }
    if dma_region_rounded_size(size) > DMA_REGION_SIZE_MAX {
        return Some(DMA_REGION_TOO_LARGE_R3);
    }
    None
}

/// The DMA region that the console's lv1 gave one call of the GameOS
/// kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDmaRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The size of the call, in bytes.
    pub size: u64,
    /// The page shift of the call.
    pub page_shift: u64,
    /// The bus address that lv1 returned in r4.
    pub address: u64,
}

/// Each region of a kernel's boot call, by firmware.
///
/// The 4.93 row is the network device's 1 MB region of 1 MB pages. The
/// capture [`DMA_REGION_CAPTURE`] finds it in the device's list, with
/// the game OS partition at +8 and the address at +48.
pub const GAME_OS_DMA_REGIONS: &[GameOsDmaRegion] = &[GameOsDmaRegion {
    firmware: "4.93",
    bus: 1,
    device: 0,
    size: 0x10_0000,
    page_shift: 20,
    address: 0xC000_0000,
}];

/// The bus address that lv1 gave the kernel's boot call for device
/// `device` of bus `bus` with `size` and `page_shift`, or `None` when no
/// capture gives that call.
pub fn game_os_dma_region(
    firmware: &str,
    bus: u64,
    device: u64,
    size: u64,
    page_shift: u64,
) -> Option<u64> {
    GAME_OS_DMA_REGIONS
        .iter()
        .find(|region| {
            region.firmware == firmware
                && region.bus == bus
                && region.device == device
                && region.size == size
                && region.page_shift == page_shift
        })
        .map(|region| region.address)
}

/// The console capture of the DMA regions that lv1 holds for the network
/// device.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_REGION_CAPTURE: &str = "tests/micro/lv1_device_dma_region";

/// The handler of hypercall 176 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaMapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `map_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 176 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_MAP_HANDLERS: &[DmaMapHandler] = &[DmaMapHandler {
    firmware: "4.93",
    handler: 0x306770,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_map_handler(firmware: &str) -> Option<&'static DmaMapHandler> {
    DMA_MAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `map_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306904-0x306910].
///
/// Bit 32 marks r3, bit 33 r4, bit 34 r5, bit 35 r6, bit 36 r7 and bit
/// 37 r8.
pub const DMA_MAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The IOPTE flags that the handler passes: bits 0 to 4 and bit 52, in
/// the bit order that bit 0 is the most significant
/// [OFW-4.93 lv1 a:0x306820-0x30682C].
pub const DMA_MAP_FLAGS_MASK: u64 = 0xF800_0000_0000_0800;

/// The r3 for flags that set a bit outside [`DMA_MAP_FLAGS_MASK`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r8 marked
/// [OFW-4.93 lv1 a:0x306830-0x30683C].
pub const DMA_MAP_FLAGS_R3: u64 = 0x0000_0020_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x236E1C-0x236E38].
pub const DMA_MAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236D88].
pub const DMA_MAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 when no region of the device holds the bus address:
/// `LV1_NO_ENTRY` (-6), with r6 marked
/// [OFW-4.93 lv1 a:0x227444-0x22744C] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_NO_REGION_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 for a length of 0, a length above the region's size, or a
/// length that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x2273E8-0x2273F0] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_LENGTH_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The r3 for a range that fails [`dma_map_range_free`]: `LV1_BUSY`
/// (-9), with r6 marked [OFW-4.93 lv1 a:0x2273F8-0x227400]
/// [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_BUSY_R3: u64 = 0x0000_0008_FFFF_FFF7;

/// The r3 for a bus address that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x227408-0x227410] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_BUS_ALIGNMENT_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 for a logical partition address that is not a multiple of the
/// page: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x227418-0x227420] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_LPAR_ALIGNMENT_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The bus addresses to which the device's entry at +120 gives an I/O
/// address [OFW-4.93 lv1 a:0x22D71C-0x22D760].
pub const DMA_MAP_IO_BUS_ADDRESSES: RangeInclusive<u64> = 0x8000_0000..=0xFFFF_FFFF;

/// Whether the range check passes the `length` bytes at `bus_address`
/// in the region of `region_size` bytes at `region_address`.
///
/// `mappings` gives the offset in the region and the length of each
/// mapping of the region. The check fails in these cases
/// [OFW-4.93 lv1 a:0x2257C4-0x225848]:
/// - the bus address is below the region;
/// - the end of the range is past the end of the region;
/// - the range overlaps a mapping.
///
/// lv1 adds and compares each value at 64 bits, unsigned.
pub fn dma_map_range_free(
    region_address: u64,
    region_size: u64,
    mappings: impl IntoIterator<Item = (u64, u64)>,
    bus_address: u64,
    length: u64,
) -> bool {
    if region_address > bus_address
        || bus_address.wrapping_add(length) > region_address.wrapping_add(region_size)
    {
        return false;
    }
    if length == 0 {
        return true;
    }
    let offset = bus_address - region_address;
    let end = offset.wrapping_add(length);
    mappings.into_iter().all(|(start, len)| {
        if offset >= start {
            start.wrapping_add(len) <= offset
        } else {
            end <= start
        }
    })
}

/// The r3 of the page checks for `bus_address`, `lpar_address` and
/// `length` in a region whose pages hold 2^`page_shift` bytes, or `None`
/// when they pass.
///
/// The checks run in this order
/// [OFW-4.93 lv1 a:0x22732C-0x227358]:
/// 1. a bus address that is not a multiple of the page gets
///    [`DMA_MAP_BUS_ALIGNMENT_R3`];
/// 2. a logical partition address that is not a multiple of the page
///    gets [`DMA_MAP_LPAR_ALIGNMENT_R3`];
/// 3. a length that is not a multiple of the page gets
///    [`DMA_MAP_LENGTH_R3`].
///
/// The page shift is one of [`DMA_REGION_PAGE_SHIFTS`], because the
/// allocate function accepts no other.
pub const fn dma_map_alignment_r3(
    bus_address: u64,
    lpar_address: u64,
    length: u64,
    page_shift: u64,
) -> Option<u64> {
    let page = 1u64 << page_shift;
    if !bus_address.is_multiple_of(page) {
        Some(DMA_MAP_BUS_ALIGNMENT_R3)
    } else if !lpar_address.is_multiple_of(page) {
        Some(DMA_MAP_LPAR_ALIGNMENT_R3)
    } else if !length.is_multiple_of(page) {
        Some(DMA_MAP_LENGTH_R3)
    } else {
        None
    }
}

/// A DMA region that lv1 holds for itself in one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv1DmaRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the region's device.
    pub bus: u64,
    /// The device id of the region's device.
    pub device: u64,
    /// The bus address of the region.
    pub address: u64,
    /// The size of the region, in bytes.
    pub size: u64,
    /// The page shift of the region.
    pub page_shift: u64,
}

/// Each region of lv1's own partition, by firmware.
///
/// The 4.93 row is region 1 of the capture [`DMA_REGION_CAPTURE`], which
/// has the partition 0 at +8.
pub const LV1_DMA_REGIONS: &[Lv1DmaRegion] = &[Lv1DmaRegion {
    firmware: "4.93",
    bus: 1,
    device: 0,
    address: 0xA000_0000,
    size: 0x8000,
    page_shift: 12,
}];

/// The region of lv1's own partition that holds `bus_address` for device
/// `device` of bus `bus`, or `None` when no capture gives one.
pub fn lv1_dma_region(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
) -> Option<&'static Lv1DmaRegion> {
    LV1_DMA_REGIONS.iter().find(|region| {
        region.firmware == firmware
            && region.bus == bus
            && region.device == device
            && region.address <= bus_address
            && bus_address - region.address < region.size
    })
}

/// The mapping that the console's lv1 made for one call of the GameOS
/// kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDmaMapping {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The bus address of the call.
    pub bus_address: u64,
    /// The logical partition address of the call.
    pub lpar_address: u64,
    /// The length of the call, in bytes.
    pub length: u64,
}

/// Each mapping of a kernel's boot call, by firmware.
///
/// The 4.93 row is the one mapping of the network device's region at
/// 0xC0000000. The capture [`DMA_MAP_CAPTURE`] finds it in the region's
/// list, with the offset 0 at +40, the address at +48 and the length at
/// +56.
pub const GAME_OS_DMA_MAPPINGS: &[GameOsDmaMapping] = &[GameOsDmaMapping {
    firmware: "4.93",
    bus: 1,
    device: 0,
    bus_address: 0xC000_0000,
    lpar_address: 0x80_0000,
    length: 0x10_0000,
}];

/// The console capture of the mapping that lv1 holds in the network
/// device's game OS region.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_MAP_CAPTURE: &str = "tests/micro/lv1_device_dma_map";

#[cfg(test)]
#[path = "tests/device_bus_open_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/device_bus_open_device_1_tests.rs"]
mod device_1_tests;

#[cfg(test)]
#[path = "tests/device_bus_dma_region_tests.rs"]
mod dma_region_tests;

#[cfg(test)]
#[path = "tests/device_bus_dma_region_console_tests.rs"]
mod dma_region_console_tests;

#[cfg(test)]
#[path = "tests/device_bus_dma_map_tests.rs"]
mod dma_map_tests;

#[cfg(test)]
#[path = "tests/device_bus_dma_map_console_tests.rs"]
mod dma_map_console_tests;
