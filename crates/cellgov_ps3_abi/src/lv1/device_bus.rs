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
//! # The storage bus
//!
//! The open function sends bus 4 to the lookup
//! [OFW-4.93 lv1 a:0x237714-0x237718]. The lookup for bus 4
//! [OFW-4.93 lv1 a:0x2365B0-0x2365B4] reads the storage bus object from
//! the global that the TOC slot at r2 - 26656 locates
//! [OFW-4.93 lv1 a:0x2365E0-0x2365E8]. The slot holds 0x368388
//! [OFW-4.93 lv1 a:0x353818-0x35381C]. The lookup returns one of these:
//! - for a device id up to 6, the object's doubleword at
//!   3816 + 8 x device [OFW-4.93 lv1 a:0x236604-0x23661C];
//! - for [`STORAGE_BUS_DEVICE`], the storage bus object itself
//!   [OFW-4.93 lv1 a:0x2365F0-0x2365FC];
//! - for any other device id, 0 [OFW-4.93 lv1 a:0x2365F4-0x2365F8].
//!
//! The last case reads nothing of the object. So for such a device id the
//! open function always returns [`OPEN_DEVICE_NO_DEVICE_R3`]; see
//! [`storage_bus_device_absent`].
//!
//! For a bus other than 1, the open function calls the method at +24 of
//! the object's method table with the object, the caller's partition and
//! the call's r5. It returns that method's r3 zero-extended
//! [OFW-4.93 lv1 a:0x23760C-0x237630].
//!
//! The object's constructor [OFW-4.93 lv1 a:0x259280] does these steps:
//! - It stores the method table at +0 [OFW-4.93 lv1 a:0x259298]
//!   [OFW-4.93 lv1 a:0x2592AC]. The TOC slot that it reads holds 0x35DD30
//!   [OFW-4.93 lv1 a:0x354670-0x354674].
//! - It sets the byte at +3496 to 1 [OFW-4.93 lv1 a:0x25929C-0x2592A4].
//! - It clears the lock word at +3576 and the record list at +3552
//!   [OFW-4.93 lv1 a:0x2592BC-0x2592DC].
//!
//! The method at +24 is the open method [OFW-4.93 lv1 a:0x35DD48-0x35DD4C]
//! [OFW-4.93 lv1 a:0x335568-0x33556C]. It holds the lock at +3576
//! [OFW-4.93 lv1 a:0x259070-0x259094] and calls the partition check
//! [OFW-4.93 lv1 a:0x2545A4] [OFW-4.93 lv1 a:0x259098-0x2590A0]. Then it
//! does one of these steps:
//! - When the check returns a nonzero status, the method returns that
//!   status sign-extended [OFW-4.93 lv1 a:0x2590A4-0x2590C4].
//! - Otherwise it allocates a 1416-byte record. When the allocation
//!   fails, it returns -2 [OFW-4.93 lv1 a:0x2590E8-0x2590F8]
//!   [OFW-4.93 lv1 a:0x259278]. The slot that the check took stays taken.
//! - Otherwise it fills the record with sixteen 80-byte entries from +88
//!   and a list of them at +1368 [OFW-4.93 lv1 a:0x2590FC-0x2591FC]. It
//!   keeps the partition at +40 of the record, links the record into the
//!   list at +3552 and returns 0 [OFW-4.93 lv1 a:0x259204-0x259258].
//!
//! The partition check does these steps:
//! 1. It reads lpar 1's `ss.laid` node for the partition
//!    [OFW-4.93 lv1 a:0x2545B8-0x2545D0] [OFW-4.93 lv1 a:0x31CE9C-0x31CEF8].
//!    A failed read gives -4 [OFW-4.93 lv1 a:0x2546EC-0x2546F8].
//! 2. It tests the byte at +3496 and the partition's authority id
//!    [OFW-4.93 lv1 a:0x2545DC-0x2545F4] [OFW-4.93 lv1 a:0x254694-0x2546E8]
//!    [OFW-4.93 lv1 a:0x25473C-0x25475C]. A failed test gives -6 or -4.
//! 3. It keeps the partition at +112 and sets the byte at +108
//!    [OFW-4.93 lv1 a:0x2545F8-0x254600].
//! 4. It holds the lock at +2536 and tests two 64-byte slots, at +2544 and
//!    then at +2608 [OFW-4.93 lv1 a:0x254604-0x254660]. A free slot ends
//!    the test, so it does not read a later slot. A used slot that holds
//!    the caller's partition gives -9 [OFW-4.93 lv1 a:0x254734]. Two used
//!    slots that hold other partitions give -2 [OFW-4.93 lv1 a:0x254658].
//! 5. Otherwise it takes the free slot: it keeps the partition at +0, the
//!    call's r5 at +8 and sets the byte at +16. It returns 0
//!    [OFW-4.93 lv1 a:0x254710-0x25472C] [OFW-4.93 lv1 a:0x254664-0x254678].
//!
//! So when the first slot is free, a partition that holds the second slot
//! takes the first slot too, and gets 0.
//!
//! Later lv1 paths test the sign of the r5 that the slot keeps
//! [OFW-4.93 lv1 a:0x254884-0x25488C] [OFW-4.93 lv1 a:0x255D30-0x255D38].
//!
//! The 4.93 kernel opens the storage bus with r3 = 4, r4 = -1 and r5 = 0,
//! and continues only when r3 is 0
//! [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae118]. The
//! capture [`STORAGE_BUS_OPEN_CAPTURE`] shows that lv1 holds the storage
//! bus open for the game OS partition after the boot. The byte at +3496
//! is 1, and the partition's slot keeps r5 = 0. The partition's record is
//! in the list. So the checks passed and the allocation succeeded for the
//! kernel's open. The first slot holds partition 1, so a second open of
//! the game OS partition gets -9.
//!
//! CellGov keeps only the r5 of the game OS partition's open. It does not
//! model the object's +108 and +112, the slot of partition 1, or the
//! record and its entries.
//!
//! # Devices 0 to 6 of the storage bus
//!
//! For a device id up to 6, the lookup returns the device object D at
//! +3816 + 8 x id of the storage bus object
//! [OFW-4.93 lv1 a:0x236604-0x23661C]. The open function then calls the
//! method at +24 of D's method table with D, the caller's partition and
//! the call's r5. It returns that method's r3 zero-extended
//! [OFW-4.93 lv1 a:0x23760C-0x237630].
//!
//! The capture [`STORAGE_DEVICE_OPEN_CAPTURE`] reads the method table of
//! each of the five devices of the 4.93 bus table. The entry at +24 of
//! each table holds the descriptor 0x334BF0:
//! - device 0, table 0x35E518 [OFW-4.93 lv1 a:0x35E530-0x35E534];
//! - device 1, table 0x35E6E8 [OFW-4.93 lv1 a:0x35E700-0x35E704];
//! - device 2, table 0x35E370 [OFW-4.93 lv1 a:0x35E388-0x35E38C];
//! - device 3, table 0x35E1C8 [OFW-4.93 lv1 a:0x35E1E0-0x35E1E4];
//! - device 4, table 0x35DFF8 [OFW-4.93 lv1 a:0x35E010-0x35E014].
//!
//! The descriptor's entry is the partition check of the storage bus
//! [OFW-4.93 lv1 a:0x334BF0-0x334BF4]. So the open of a storage device is
//! that check on D, with no record after it. The check returns its status
//! sign-extended [OFW-4.93 lv1 a:0x254678], and the open function clears
//! the high word.
//!
//! The check's second step passes for D when these tests pass:
//! - D's byte at +3496 is set [OFW-4.93 lv1 a:0x2545DC-0x2545E4];
//! - the doubleword at +2352 of the object at D + 144 is not 0, or the
//!   method at +296 of D's table returns a value that is not 0
//!   [OFW-4.93 lv1 a:0x2545E8-0x2545F4] [OFW-4.93 lv1 a:0x254694-0x2546B4].
//!
//! When either test fails, the partition's authority id decides. A
//! failed second test passes for two authority ids
//! [OFW-4.93 lv1 a:0x2546B8-0x2546E8]. A clear byte at +3496 lets one
//! authority id go on to the second test, and the check returns -6 for
//! each other id [OFW-4.93 lv1 a:0x25473C-0x25475C].
//!
//! The capture reads these values after the boot to the XMB:
//! - Each device's byte at +3496 is 1, its byte at +108 is 1 and its
//!   +112 holds partition 2.
//! - For devices 0, 2 and 3, slot 0 holds partition 2 with r5 =
//!   0x8000000000000000, and slot 1 is free.
//! - For devices 1 and 4, slot 0 holds partition 1 with r5 = 0, and
//!   slot 1 holds partition 2 with r5 = 0x8000000000000000.
//!
//! The check writes a slot only on the path that returns 0. So the game
//! OS partition opened each device with r5 = 0x8000000000000000, and the
//! check passed for each open. A second open of the game OS partition
//! finds its own slot, and gets -9.
//!
//! The 4.93 kernel's device object opens device +152 of bus +144 with
//! r5 = 0x8000000000000000
//! [OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac120]. The
//! object's constructor copies the bus id and the device id from +0 and
//! +8 of the kernel's device record
//! [OFW-4.93 lv2_kernel a:0x80000000000a98c8-0x80000000000a98e8].
//!
//! CellGov keeps only the device id and the r5 of each game OS open. It
//! does not model the slot of partition 1 or D's +108 and +112.
//!
//! # `close_device`
//!
//! The handler [OFW-4.93 lv1 a:0x306EEC] does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`CLOSE_DEVICE_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306F74-0x306F78]
//!    [OFW-4.93 lv1 a:0x30701C-0x30702C].
//! 2. It calls the close function [OFW-4.93 lv1 a:0x2373B0] with the
//!    caller's partition, r3 and r4 [OFW-4.93 lv1 a:0x306F7C-0x306F94].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x306F98]
//!    [OFW-4.93 lv1 a:0x307000]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The close function does these steps. The first failure is the answer:
//! 1. It runs the access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x2373E4-0x2373EC], which
//!    [`network`](super::network) describes. The function zero-extends a
//!    failed status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x237520-0x237540]. See [`close_device_check_r3`].
//! 2. It looks up the device [OFW-4.93 lv1 a:0x2373F0-0x237408]. The
//!    lookup finds the device that the access check found.
//! 3. Bus 1 takes a path of its own for each of the devices 0, 1 and 2
//!    [OFW-4.93 lv1 a:0x23740C-0x237410] [OFW-4.93 lv1 a:0x237458-0x23745C]
//!    [OFW-4.93 lv1 a:0x2374F8-0x237504]. For each other device, the
//!    function calls the method at +32 of the device's method table, the
//!    close entry, with the partition. It returns that r3 zero-extended
//!    [OFW-4.93 lv1 a:0x237414-0x237434].
//!
//! The access check runs before each step that changes state. So a close
//! of a device that the caller does not hold open changes nothing.
//!
//! ## Device 0 of bus 1
//!
//! The close function holds the lock at +40 of the network device's
//! global object [OFW-4.93 lv1 a:0x237460-0x237498] and does these steps:
//! 1. It runs the close hook [OFW-4.93 lv1 a:0x23E240] with the global
//!    object and the object at +16 [OFW-4.93 lv1 a:0x23749C-0x2374A4]. A
//!    nonzero status of the hook is the answer, zero-extended, and the
//!    device stays open [OFW-4.93 lv1 a:0x2374A8-0x2374B0].
//! 2. Otherwise it calls the device's close entry with the partition. It
//!    returns 0, or the entry's nonzero r3 zero-extended
//!    [OFW-4.93 lv1 a:0x237544-0x237578].
//!
//! The hook does these steps:
//! - An absent object at +16 gives -6 [OFW-4.93 lv1 a:0x23E240-0x23E264].
//! - A state word at +8 other than 2 gives 0 and changes nothing
//!   [OFW-4.93 lv1 a:0x23E268-0x23E278].
//! - A state word of 2 leads to the teardown [OFW-4.93 lv1 a:0x23DFA8] on
//!   each path [OFW-4.93 lv1 a:0x23E294-0x23E2F4].
//!
//! The open of device 0 sets the state word to 2, so the hook tears down
//! the object of a device that the caller holds open.
//!
//! The teardown does these steps on the object:
//! 1. It calls a stop function [OFW-4.93 lv1 a:0x23DFA8-0x23DFD8]. That
//!    function fails with -24 only when a poll of a hardware register stops
//!    after 1000 tries [OFW-4.93 lv1 a:0x243924-0x243968]
//!    [OFW-4.93 lv1 a:0x2439C4]. On that failure the teardown returns the
//!    status, and the state word stays 2 [OFW-4.93 lv1 a:0x23DFDC-0x23E00C].
//! 2. It stops the device's register block [OFW-4.93 lv1 a:0x23E010-0x23E014].
//! 3. When the object's flag byte at +0 is set, it disconnects the event
//!    receive port of the device's interrupt
//!    [OFW-4.93 lv1 a:0x23E018-0x23E020] [OFW-4.93 lv1 a:0x23E128-0x23E138].
//!    The connection is the object of `connect_interrupt_event_receive_port`
//!    [OFW-4.93 lv1 a:0x353CE8-0x353CEC] [OFW-4.93 lv1 a:0x353838-0x35383C].
//!    The disconnect function drops the connection's port
//!    [OFW-4.93 lv1 a:0x245780-0x245848].
//! 4. It destroys the 288-byte object at +24 [OFW-4.93 lv1 a:0x23E024-0x23E044].
//! 5. It stores 0 in the interrupt mask at +32, -1 in the status indicator
//!    at +48 and 0 at +24 [OFW-4.93 lv1 a:0x23E048-0x23E060].
//! 6. When +56 holds the memory region of the status indicator, it
//!    decrements the region's count at +192 and clears +56
//!    [OFW-4.93 lv1 a:0x23E064-0x23E0F4].
//! 7. It stores 3 in the state word and returns 0
//!    [OFW-4.93 lv1 a:0x23E0F8-0x23E108].
//!
//! The capture [`NET_CONTROL_CAPTURE`](super::network::NET_CONTROL_CAPTURE)
//! reads 1 in the flag byte of the open device. No input of the call
//! selects the failure of step 1.
//!
//! Device 0's close entry [OFW-4.93 lv1 a:0x2294D4]
//! [OFW-4.93 lv1 a:0x35C898-0x35C89C] [OFW-4.93 lv1 a:0x32D6A8-0x32D6AC]
//! holds the device's lock at +104 and does these steps:
//! 1. A clear open flag at +108, or a partition at +112 other than the
//!    caller's, gives -11 [OFW-4.93 lv1 a:0x22952C-0x229588].
//! 2. It removes the caller's DMA mappings [OFW-4.93 lv1 a:0x226A58]
//!    [OFW-4.93 lv1 a:0x22958C-0x229598]; see below.
//! 3. It frees each DMA region that the caller holds
//!    [OFW-4.93 lv1 a:0x229594-0x229650]; see below.
//! 4. It removes the caller's MMIO mappings [OFW-4.93 lv1 a:0x2292EC]
//!    [OFW-4.93 lv1 a:0x229680-0x229688].
//! 5. It stores -1 at +112, clears the open flag and returns 0
//!    [OFW-4.93 lv1 a:0x22968C-0x2296A0].
//!
//! The unmap step walks each region of the list at +96, lv1's own region
//! too [OFW-4.93 lv1 a:0x226AAC-0x226AC0] [OFW-4.93 lv1 a:0x226B34-0x226B3C].
//! It acts on each mapping whose word at +88 is 0 and whose partition at
//! +8 is the caller's [OFW-4.93 lv1 a:0x226AE0-0x226AF8]. For a partition
//! other than 0, it calls the device's entry at +184 with the bus address
//! of the mapping [OFW-4.93 lv1 a:0x226B78-0x226BA4]
//! [OFW-4.93 lv1 a:0x35C930-0x35C934] [OFW-4.93 lv1 a:0x32D468-0x32D46C].
//! That entry does these steps:
//! - It gets the I/O address of the bus address from the entry at +120,
//!   and returns that entry's failure at once
//!   [OFW-4.93 lv1 a:0x2279F8-0x227A28].
//! - It clears the IOPTEs with the entry at +136
//!   [OFW-4.93 lv1 a:0x227A2C-0x227AA8].
//! - On each path after the IOPTE step, it unlinks the mapping from the
//!   region and destroys it, whether that step failed or not
//!   [OFW-4.93 lv1 a:0x227AAC-0x227AC0] [OFW-4.93 lv1 a:0x227AEC-0x227B30]
//!   [OFW-4.93 lv1 a:0x22584C-0x2258B4].
//!
//! The free step calls the device's free method
//! [OFW-4.93 lv1 a:0x227650] [OFW-4.93 lv1 a:0x35C938-0x35C93C]
//! [OFW-4.93 lv1 a:0x32D408-0x32D40C] for each region whose partition at
//! +8 is the caller's [OFW-4.93 lv1 a:0x2295E8-0x22960C]:
//! - A region that still holds a mapping gets -9 and stays
//!   [OFW-4.93 lv1 a:0x2276B8-0x2276C4]. The close entry skips it
//!   [OFW-4.93 lv1 a:0x229610-0x229614] [OFW-4.93 lv1 a:0x2296A4-0x2296AC].
//! - Otherwise the method releases the region's bus address, unlinks the
//!   region from the list at +96 and destroys it
//!   [OFW-4.93 lv1 a:0x2276C8-0x227768].
//!
//! The entry at +120 gave each mapping of the partition its I/O address
//! when `map_device_dma_region` made the mapping. It used the same bus
//! address and the same device word at +160 then. The mapping function stores
//! 0 at +88, as the capture [`DMA_MAP_CAPTURE`] reads. So the unmap step
//! removes each mapping of the partition, and the free step then frees
//! each region of the partition.
//!
//! ## Devices 1 and 2 of bus 1
//!
//! The close function calls the close of the controller in slot 0 for
//! device 1 and in slot 1 for device 2. It returns that close's r3
//! zero-extended [OFW-4.93 lv1 a:0x23757C-0x23758C]
//! [OFW-4.93 lv1 a:0x237508-0x237518]. The controller's close holds the
//! controller's lock [OFW-4.93 lv1 a:0x23D854-0x23D87C] and does these
//! steps:
//! - When the state word at +240 is not 0 and the device's partition is
//!   the caller's, it clears each flagged slot of the 6 slots at +104. It
//!   then writes 0 to the register at +0xF00 of the controller's register
//!   block [OFW-4.93 lv1 a:0x23D880-0x23D898]
//!   [OFW-4.93 lv1 a:0x23D904-0x23D980].
//! - When the state word is not 0, it stores 3 in it
//!   [OFW-4.93 lv1 a:0x23D89C-0x23D8A0] [OFW-4.93 lv1 a:0x23D8FC-0x23D900].
//! - It calls the device's close entry and returns its r3 sign-extended
//!   [OFW-4.93 lv1 a:0x23D8A4-0x23D8D4].
//!
//! The device's method table is the table of device 0, so the close entry
//! is the close entry of device 0.
//!
//! After a close, the controller's next open writes the controller's
//! registers before it sets the state word to 2
//! [OFW-4.93 lv1 a:0x23DD2C-0x23DD94]. Those registers are in the
//! controller's register block, outside partition memory. That open
//! cannot get -5, for the reason that the section on device 1 of bus 1
//! gives.
//!
//! ## The storage bus
//!
//! For device -1 of bus 4, the close entry is the storage bus object's
//! close method [OFW-4.93 lv1 a:0x259BD8] [OFW-4.93 lv1 a:0x35DD50-0x35DD54]
//! [OFW-4.93 lv1 a:0x335640-0x335644]. Its gate calls the object's method
//! at +48, and then its methods at +232 and +240, which read the state of
//! the storage devices [OFW-4.93 lv1 a:0x253FA0-0x254050]. The method
//! then calls a method of the partition's record through the record's
//! own table [OFW-4.93 lv1 a:0x259CC0-0x259CD4]. CellGov does not model
//! that state.
//!
//! # The 4.93 kernel's calls of `close_device`
//!
//! The kernel sends hypercall 171 at these sites:
//! - with the bus id and the device id at +16 and +8 of a device record
//!   [OFW-4.93 lv2_kernel a:0x80000000000A3570-0x80000000000A357C];
//! - with the bus id and the device id at +144 and +152 of a device record
//!   [OFW-4.93 lv2_kernel a:0x80000000000A90F8-0x80000000000A9104]
//!   [OFW-4.93 lv2_kernel a:0x80000000000AC2C4-0x80000000000AC2D0];
//! - for the storage bus, with r4 = -1
//!   [OFW-4.93 lv2_kernel a:0x80000000000AD594-0x80000000000AD5B4]
//!   [OFW-4.93 lv2_kernel a:0x80000000000AE2F0-0x80000000000AE2FC].
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
//!
//! # `map_device_mmio_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306D64] keeps r3 to r7
//! [OFW-4.93 lv1 a:0x306D78] [OFW-4.93 lv1 a:0x306D98-0x306DA4] and does
//! these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`MMIO_MAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306E04-0x306E08]
//!    [OFW-4.93 lv1 a:0x306ED8-0x306EE8].
//! 2. It zeroes the return slot of r4, HSPRG0 - 0x6A38, and calls the map
//!    function [OFW-4.93 lv1 a:0x237254] with the caller's partition, r3
//!    to r7 and the address of that slot
//!    [OFW-4.93 lv1 a:0x306E0C-0x306E44].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x306E48]
//!    [OFW-4.93 lv1 a:0x306EB0]. The vector's entry zeroes the other
//!    slots [OFW-4.93 lv1 a:0x0C08], so r5 to r10 return 0.
//!
//! The map function writes the slot only on success
//! [OFW-4.93 lv1 a:0x237300-0x237308], so a failure returns r4 = 0. It
//! does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x2372AC-0x2372B4], which
//!    [`network`](super::network) describes. Bus 0 gets
//!    [`MMIO_MAP_NO_BUS_R3`]. A partition that did not open the device
//!    gets [`MMIO_MAP_NOT_OPEN_R3`]. The function zero-extends a failed
//!    status and sets bit (mark + 31) for a mark other than 0
//!    [OFW-4.93 lv1 a:0x237340-0x237390].
//! 2. Device 0 of bus 1 gets [`MMIO_MAP_DEVICE_0_R3`]
//!    [OFW-4.93 lv1 a:0x2372B8-0x2372BC] [OFW-4.93 lv1 a:0x237394-0x2373AC].
//! 3. The device lookup [OFW-4.93 lv1 a:0x2372C0-0x2372D4]. For bus 1 it
//!    is the lookup of the access check, so it finds the device.
//! 4. The device's MMIO map function [OFW-4.93 lv1 a:0x228BEC], with the
//!    device, the partition, r5, r6, r7, two outputs and a mark
//!    [OFW-4.93 lv1 a:0x2372D8-0x2372F4]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x2372F8-0x2372FC]. On 0 the function
//!    stores the logical partition address in the slot of r4 and returns
//!    0 [OFW-4.93 lv1 a:0x237300-0x237308].
//!
//! The device's MMIO map function holds the device's lock at +16
//! [OFW-4.93 lv1 a:0x228C48-0x228C6C] and does these steps:
//! 1. It walks the 8 region slots at +24 of the device for the region
//!    whose bus address at +48 is r5 [OFW-4.93 lv1 a:0x228C70-0x228CA0].
//!    When no region has that address, the answer is
//!    [`MMIO_MAP_NO_REGION_R3`] [OFW-4.93 lv1 a:0x228CA4-0x228CAC].
//! 2. A size of 0 or above the region's size at +56 gets
//!    [`MMIO_MAP_SIZE_R3`] [OFW-4.93 lv1 a:0x228D04-0x228D14]
//!    [OFW-4.93 lv1 a:0x228DF4-0x228E00].
//! 3. A page shift other than those of [`MMIO_MAP_PAGE_SHIFTS`] gets
//!    [`MMIO_MAP_PAGE_SHIFT_R3`] [OFW-4.93 lv1 a:0x228D18-0x228D34]
//!    [OFW-4.93 lv1 a:0x228DE4-0x228DF0].
//! 4. It calls the device's entry at +128 with the bus address
//!    [OFW-4.93 lv1 a:0x228D38-0x228D5C] [OFW-4.93 lv1 a:0x35C8F8-0x35C8FC]
//!    [OFW-4.93 lv1 a:0x32E0F8-0x32E0FC] [OFW-4.93 lv1 a:0x22D9C0-0x22D9D0].
//!    That entry gives the real address of the bus address, and returns
//!    0 on each path [OFW-4.93 lv1 a:0x22BE1C-0x22BE68].
//! 5. It calls the address function [OFW-4.93 lv1 a:0x229CA8] with the
//!    partition, the real address, the page shift, the size, an output
//!    and the mark [OFW-4.93 lv1 a:0x228D68-0x228D80]. A nonzero r3 of
//!    that function is the answer [OFW-4.93 lv1 a:0x228D84-0x228D8C].
//! 6. It makes the mapping with the mapping function of the DMA path
//!    [OFW-4.93 lv1 a:0x225E18], with the region, the partition, the
//!    region's bus address, the size, the logical partition address and 1
//!    in r9 and r10 [OFW-4.93 lv1 a:0x228D90-0x228DB8].
//! 7. It clears the mark, stores the logical partition address at +80 of
//!    the mapping, gives that address to the caller and returns 0
//!    [OFW-4.93 lv1 a:0x228DC8-0x228DE0] [OFW-4.93 lv1 a:0x228CB0-0x228CC4].
//!
//! The address function takes the logical partition address from an
//! allocator [OFW-4.93 lv1 a:0x229E78-0x229E80], and gives it only on
//! success [OFW-4.93 lv1 a:0x229F84-0x229F98]. The allocator's state is
//! lv1's own, which CellGov does not model. [`GAME_OS_MMIO_MAPPINGS`]
//! gives the address that the console's lv1 gave the 4.93 kernel's call.
//!
//! With r10 not 0, the mapping function skips the range check
//! [OFW-4.93 lv1 a:0x225E18] [OFW-4.93 lv1 a:0x225E5C]. That check
//! refuses a range outside the region
//! [OFW-4.93 lv1 a:0x225F7C-0x225F84] [OFW-4.93 lv1 a:0x225FE4-0x225FF4]
//! and a range that overlaps a mapping of the region
//! [OFW-4.93 lv1 a:0x225FF8-0x226048]. So a mapping of lv1's own in the
//! region does not stop the call. The function keeps r9
//! in the byte at +64 of the mapping [OFW-4.93 lv1 a:0x225E9C]. When its
//! 96-byte allocation fails [OFW-4.93 lv1 a:0x225E60-0x225E74], the map
//! function frees the logical partition address and returns
//! `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x228DC4]
//! [OFW-4.93 lv1 a:0x228E20-0x228E34]. No input of the call selects that
//! case.
//!
//! lv1's init gives device 1 of bus 1 the three regions of
//! [`MMIO_REGIONS`] [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C]. The region
//! function stores region i in the slot at +24 + 8 x i. It makes each
//! region an 80-byte object with the page shift 20
//! [OFW-4.93 lv1 a:0x227F4C-0x227FC8]. The capture [`MMIO_MAP_CAPTURE`]
//! reads these regions in the device's slots.
//!
//! The close entry that devices 0, 1 and 2 of bus 1 share removes the
//! caller's MMIO mappings [OFW-4.93 lv1 a:0x2292EC]
//! [OFW-4.93 lv1 a:0x229680-0x229688]. For each region of the device, it
//! frees the logical partition address of each mapping of the caller,
//! and unlinks and destroys the mapping [OFW-4.93 lv1 a:0x229360-0x229448].
//!
//! # The 4.93 kernel's call of `map_device_mmio_region`
//!
//! The kernel's wrapper keeps the bus id, the device id and the size from
//! r3, r4 and r6 [OFW-4.93 lv2_kernel a:0x800000000007D860-0x800000000007D868].
//! For a flags word in r7 that sets none of the bits it tests, it sends
//! the call with its own r3 to r6 and r7 = 12
//! [OFW-4.93 lv2_kernel a:0x800000000007D894-0x800000000007D8CC]. When r3
//! is 0, it keeps r4 as the logical partition address
//! [OFW-4.93 lv2_kernel a:0x800000000007D8D0-0x800000000007D8DC]. The
//! capture [`MMIO_MAP_CAPTURE`] gives lv1's record of the call; see
//! [`GAME_OS_MMIO_MAPPINGS`].

use std::ops::RangeInclusive;

use super::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_WRONG_STATE};
use super::storage::STORAGE_DEVICE_ID_MAX;
use super::storage_registration::registration;

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

/// The bus id of the storage bus, which the lookup reads from the storage
/// bus object [OFW-4.93 lv1 a:0x2365B0-0x2365B4].
pub const STORAGE_BUS: u64 = 4;

/// The device id that names the storage bus object itself, -1
/// [OFW-4.93 lv1 a:0x2365F0-0x2365FC].
pub const STORAGE_BUS_DEVICE: u64 = u64::MAX;

/// Whether `open_device` of device `device` on bus `bus` opens the storage
/// bus object.
pub const fn opens_storage_bus(bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS && device == STORAGE_BUS_DEVICE
}

/// Whether the lookup finds no device for device `device` of bus `bus`
/// whatever the storage bus object holds.
///
/// For bus [`STORAGE_BUS`], a device id above
/// [`STORAGE_DEVICE_ID_MAX`] other than [`STORAGE_BUS_DEVICE`]
/// gets 0 from the lookup [OFW-4.93 lv1 a:0x2365F0-0x2365F8]. The open
/// function then returns [`OPEN_DEVICE_NO_DEVICE_R3`]
/// [OFW-4.93 lv1 a:0x237600] [OFW-4.93 lv1 a:0x237780-0x237790].
pub const fn storage_bus_device_absent(bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS && device > STORAGE_DEVICE_ID_MAX && device != STORAGE_BUS_DEVICE
}

/// The r3 of an open of the storage bus that the partition holds open:
/// `LV1_BUSY` (-9), zero-extended.
///
/// The partition check returns -9 [OFW-4.93 lv1 a:0x254734]. The open
/// method sign-extends it [OFW-4.93 lv1 a:0x2590C4], and the open function
/// clears the high word [OFW-4.93 lv1 a:0x237630].
pub const OPEN_STORAGE_BUS_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The read-only lv1 capture of the storage bus object after the 4.93
/// boot, which shows the game OS partition's open of the storage bus.
pub const STORAGE_BUS_OPEN_CAPTURE: &str = "tests/micro/lv1_storage_bus_open";

/// The read-only lv1 capture of the storage device objects after the 4.93
/// boot, which shows the game OS partition's open of each device.
pub const STORAGE_DEVICE_OPEN_CAPTURE: &str = "tests/micro/lv1_storage_device_open";

/// Whether `open_device` of device `device` on bus `bus` at the firmware
/// version `firmware` opens a storage device whose open path CellGov read.
///
/// These devices are the devices of the bus table of [`registration`] at
/// `firmware`. [`STORAGE_DEVICE_OPEN_CAPTURE`] reads the device object of
/// each 4.93 device.
pub fn opens_storage_device(firmware: &str, bus: u64, device: u64) -> bool {
    bus == STORAGE_BUS
        && device <= STORAGE_DEVICE_ID_MAX
        && registration(firmware).is_some_and(|table| table.device(device).is_some())
}

/// The r3 of an open of a storage device that the partition holds open:
/// `LV1_BUSY` (-9), zero-extended.
///
/// The partition check returns -9 [OFW-4.93 lv1 a:0x254734] sign-extended
/// [OFW-4.93 lv1 a:0x254678], and the open function clears the high word
/// [OFW-4.93 lv1 a:0x237630].
pub const OPEN_STORAGE_DEVICE_BUSY_R3: u64 = zero_extended(LV1_BUSY);

/// The handler of hypercall 171 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseDeviceHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `close_device` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 171 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const CLOSE_DEVICE_HANDLERS: &[CloseDeviceHandler] = &[CloseDeviceHandler {
    firmware: "4.93",
    handler: 0x306EEC,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn close_device_handler(firmware: &str) -> Option<&'static CloseDeviceHandler> {
    CLOSE_DEVICE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `close_device` for a bus id above [`OPEN_DEVICE_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x30701C-0x30702C].
pub const CLOSE_DEVICE_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `close_device` for bus id 0: `LV1_NO_ENTRY` (-6), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x236420-0x236428]
/// [OFW-4.93 lv1 a:0x237520-0x237540].
pub const CLOSE_DEVICE_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `close_device` when the access check finds no device:
/// `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x236464-0x236474] [OFW-4.93 lv1 a:0x237520-0x237540].
pub const CLOSE_DEVICE_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `close_device` for a device that the caller does not hold
/// open: `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x237520-0x23752C].
pub const CLOSE_DEVICE_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 of `close_device`'s checks that read no state, for device
/// `device` of bus `bus`, or `None` when the call passes them.
///
/// The checks run in this order:
/// 1. a bus id above [`OPEN_DEVICE_BUS_MAX`] gets
///    [`CLOSE_DEVICE_BUS_RANGE_R3`];
/// 2. bus id 0 gets [`CLOSE_DEVICE_NO_BUS_R3`];
/// 3. a device of bus [`STORAGE_BUS`] that
///    [`storage_bus_device_absent`] names gets
///    [`CLOSE_DEVICE_NO_DEVICE_R3`]. The access check finds no device for
///    it [OFW-4.93 lv1 a:0x2364E8-0x2364F4]
///    [OFW-4.93 lv1 a:0x236554-0x236558].
pub const fn close_device_check_r3(bus: u64, device: u64) -> Option<u64> {
    if bus > OPEN_DEVICE_BUS_MAX {
        Some(CLOSE_DEVICE_BUS_RANGE_R3)
    } else if bus == 0 {
        Some(CLOSE_DEVICE_NO_BUS_R3)
    } else if storage_bus_device_absent(bus, device) {
        Some(CLOSE_DEVICE_NO_DEVICE_R3)
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
/// the bit order in which bit 0 is the most significant
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
/// holds partition 0 at +8.
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

/// The handler of hypercall 172 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioMapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `map_device_mmio_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 172 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const MMIO_MAP_HANDLERS: &[MmioMapHandler] = &[MmioMapHandler {
    firmware: "4.93",
    handler: 0x306D64,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn mmio_map_handler(firmware: &str) -> Option<&'static MmioMapHandler> {
    MMIO_MAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `map_device_mmio_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306ED8-0x306EE4].
///
/// Bit 32 marks r3, bit 33 r4, bit 34 r5, bit 35 r6 and bit 36 r7.
pub const MMIO_MAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x237340-0x23734C].
pub const MMIO_MAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for device 0 of bus 1, which the partition holds open:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x237394-0x2373AC].
pub const MMIO_MAP_DEVICE_0_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 when no region of the device has the bus address:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x228CA4-0x228CAC] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_NO_REGION_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a size of 0 or above the region's size:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x228DF4-0x228E00] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_SIZE_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 for a page shift outside [`MMIO_MAP_PAGE_SHIFTS`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x228DE4-0x228DF0] [OFW-4.93 lv1 a:0x237340-0x237390].
pub const MMIO_MAP_PAGE_SHIFT_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The page shifts that the device's MMIO map function accepts
/// [OFW-4.93 lv1 a:0x228D18-0x228D34].
pub const MMIO_MAP_PAGE_SHIFTS: [u64; 4] = [12, 16, 20, 24];

/// An MMIO region that lv1's init gives a device in one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the region's device.
    pub bus: u64,
    /// The device id of the region's device.
    pub device: u64,
    /// The index of the region's slot in the device.
    pub slot: u64,
    /// The bus address of the region.
    pub address: u64,
    /// The size of the region, in bytes.
    pub size: u64,
}

/// Each MMIO region of device 1 of bus 1, by firmware, in slot order.
///
/// lv1's init gives the 4.93 regions [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C].
/// The capture [`MMIO_MAP_CAPTURE`] reads them in slots 0 to 2 of the
/// device, and reads the other 5 slots empty.
pub const MMIO_REGIONS: &[MmioRegion] = &[
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 0,
        address: 0x2400,
        size: 0x200,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 1,
        address: 0x301_0000,
        size: 0x1_0000,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 2,
        address: 0x381_0000,
        size: 0x1_0000,
    },
];

/// Whether a source gives every MMIO region of device `device` of bus
/// `bus` at the firmware version `firmware`.
pub fn mmio_regions_read(firmware: &str, bus: u64, device: u64) -> bool {
    MMIO_REGIONS
        .iter()
        .any(|region| region.firmware == firmware && region.bus == bus && region.device == device)
}

/// The MMIO region of device `device` of bus `bus` whose bus address is
/// `bus_address`, or `None` when the device has no such region.
///
/// The device's MMIO map function compares the address for equality
/// [OFW-4.93 lv1 a:0x228C90-0x228C98].
pub fn mmio_region(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
) -> Option<&'static MmioRegion> {
    MMIO_REGIONS.iter().find(|region| {
        region.firmware == firmware
            && region.bus == bus
            && region.device == device
            && region.address == bus_address
    })
}

/// The MMIO mapping that the console's lv1 made for one call of the
/// GameOS kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsMmioMapping {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The bus address of the call.
    pub bus_address: u64,
    /// The size of the call, in bytes.
    pub size: u64,
    /// The page shift of the call.
    pub page_shift: u64,
    /// The logical partition address that lv1 gave the call.
    pub lpar_address: u64,
}

/// Each MMIO mapping of a kernel's boot call, by firmware.
///
/// The 4.93 row is the one game OS mapping of the region in slot 1 of
/// device 1. The
/// capture [`MMIO_MAP_CAPTURE`] finds it in the region's list, with the
/// partition 2 at +8, the address at +48 and +80 and the length at +56.
/// The page shift is the one that the kernel's wrapper sends
/// [OFW-4.93 lv2_kernel a:0x800000000007D8B0].
pub const GAME_OS_MMIO_MAPPINGS: &[GameOsMmioMapping] = &[GameOsMmioMapping {
    firmware: "4.93",
    bus: 1,
    device: 1,
    bus_address: 0x301_0000,
    size: 0x1_0000,
    page_shift: 12,
    lpar_address: 0x4000_0007_0000,
}];

/// The console's record of the call that names `bus`, `device`,
/// `bus_address`, `size` and `page_shift`, or `None` when no capture
/// gives one.
pub fn game_os_mmio_mapping(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
    size: u64,
    page_shift: u64,
) -> Option<&'static GameOsMmioMapping> {
    GAME_OS_MMIO_MAPPINGS.iter().find(|row| {
        row.firmware == firmware
            && row.bus == bus
            && row.device == device
            && row.bus_address == bus_address
            && row.size == size
            && row.page_shift == page_shift
    })
}

/// The console capture of the MMIO regions of device 1 of bus 1 and the
/// mappings that lv1 holds in its second region.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const MMIO_MAP_CAPTURE: &str = "tests/micro/lv1_device_mmio_map";

#[cfg(test)]
#[path = "tests/device_bus_open_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/device_bus_open_device_1_tests.rs"]
mod device_1_tests;

#[cfg(test)]
#[path = "tests/device_bus_storage_bus_tests.rs"]
mod storage_bus_tests;

#[cfg(test)]
#[path = "tests/device_bus_absent_device_tests.rs"]
mod absent_device_tests;

#[cfg(test)]
#[path = "tests/device_bus_close_tests.rs"]
mod close_tests;

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

#[cfg(test)]
#[path = "tests/device_bus_mmio_map_tests.rs"]
mod mmio_map_tests;

#[cfg(test)]
#[path = "tests/device_bus_mmio_map_console_tests.rs"]
mod mmio_map_console_tests;
