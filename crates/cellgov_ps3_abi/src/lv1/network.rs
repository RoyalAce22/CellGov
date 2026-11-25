//! The facts of the network device hypercalls.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! # `net_set_interrupt_status_indicator`
//!
//! The handler [OFW-4.93 lv1 a:0x308204] keeps r3 to r6
//! [OFW-4.93 lv1 a:0x308218] [OFW-4.93 lv1 a:0x308234-0x30823C] and
//! checks them in this order. The first failure is the answer:
//! 1. a bus id above [`STATUS_INDICATOR_BUS_MAX`] gets
//!    [`STATUS_INDICATOR_BUS_RANGE_R3`];
//! 2. a bus id other than [`STATUS_INDICATOR_BUS`] gets
//!    [`STATUS_INDICATOR_NO_BUS_R3`];
//! 3. a device id other than [`STATUS_INDICATOR_DEVICE`] gets
//!    [`STATUS_INDICATOR_NO_DEVICE_R3`];
//! 4. the network device's state word, see below;
//! 5. an r6 other than 0 gets [`STATUS_INDICATOR_R6_R3`];
//! 6. the address in r5, see [`STATUS_INDICATOR_ADDRESS_R3`];
//! 7. an indicator that is set gets [`STATUS_INDICATOR_SET_R3`].
//!
//! The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x3083A8]. It writes no return slot, so r4 to r10
//! return 0.
//!
//! # The network device's state word
//!
//! For device 0 of bus 1, the handler takes the lock at +40 of a global
//! object, and reads the object at +16 of it
//! [OFW-4.93 lv1 a:0x3082B4-0x3082F0]. A state word at +8 of that object
//! other than 2 gives [`STATUS_INDICATOR_WRONG_STATE_R3`]
//! [OFW-4.93 lv1 a:0x3082F4-0x308308].
//!
//! `open_device` for device 0 of bus 1 sets that state word to 2:
//! - The handler loads the global object from the TOC slot at r2 - 1848
//!   [OFW-4.93 lv1 a:0x3082B4] [OFW-4.93 lv1 a:0x359900]. The open
//!   function loads it from the TOC slot at r2 - 26632
//!   [OFW-4.93 lv1 a:0x237660] [OFW-4.93 lv1 a:0x353830]. The two slots
//!   hold the same address.
//! - After the device's open entry returns 0, the open function runs its
//!   hook with the global object and the object at +16 of it
//!   [OFW-4.93 lv1 a:0x237794-0x2377A0].
//! - Each path of the hook that does not fail reaches the function
//!   [OFW-4.93 lv1 a:0x23E174] [OFW-4.93 lv1 a:0x23E34C-0x23E3AC]. That
//!   function sets the state word to 2 when its setup succeeds
//!   [OFW-4.93 lv1 a:0x23E1BC-0x23E1D4].
//! - When the hook fails, the open function closes the device again and
//!   returns the hook's status [OFW-4.93 lv1 a:0x2377B4-0x2377D8].
//!
//! So a device 0 of bus 1 that `open_device` opened has the state word 2.
//!
//! `close_device` of an open device 0 of bus 1 runs the teardown on the
//! object at +16; see the `close_device` section of
//! [`super::device_bus`]. The teardown does these stores
//! [OFW-4.93 lv1 a:0x23E048-0x23E060] [OFW-4.93 lv1 a:0x23E0F8-0x23E108]:
//! - 0 in the mask at +32;
//! - -1 in the indicator at +48;
//! - [`TORN_DOWN_STATE_WORD`] in the state word.
//!
//! It destroys only the object at +24 of that object
//! [OFW-4.93 lv1 a:0x23E024-0x23E044]. The close hook reads the doubleword
//! at +16 of the global object and writes no field of the global object
//! [OFW-4.93 lv1 a:0x23E240-0x23E2F4]. So after a close, the object at +16
//! stays. Each of these calls then reads the state word 3 and gets its
//! wrong state r3:
//! - `net_set_interrupt_status_indicator`;
//! - `net_control`;
//! - `net_set_interrupt_mask`.
//!
//! A later `open_device` of the device runs the open hook. The hook
//! refuses only the state word 2 [OFW-4.93 lv1 a:0x23E33C-0x23E348], so it
//! reaches the setup [OFW-4.93 lv1 a:0x23E34C-0x23E370]. The reopen does
//! these steps on the object at +16:
//! - The setup stores a new object at +24 and the state word 2
//!   [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4] [OFW-4.93 lv1 a:0x23E1F4].
//! - The setup's callee keeps the object at +16 only as a pointer in the
//!   new object [OFW-4.93 lv1 a:0x242874].
//! - On one path the hook first calls [OFW-4.93 lv1 a:0x2482BC]. That
//!   function reads only r3, the object that the doubleword at 0x368328
//!   points to [OFW-4.93 lv1 a:0x23E3A0-0x23E3A8]
//!   [OFW-4.93 lv1 a:0x353D20-0x353D24]. The global object is at the
//!   doubleword 0x3682B0.
//!
//! So no step of the reopen stores the mask at +32 or the indicator at
//! +48. After the reopen the mask stays 0 and no indicator is set.
//!
//! # The address
//!
//! For an r5 that passes the alignment check, the handler does these
//! steps:
//! 1. It looks up the memory region that holds r5 with the function
//!    [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x30841C-0x308458]; see
//!    [`super::interrupt::region_lookup_base`].
//! 2. It checks the region's class at +72 against the TOC slot at
//!    r2 - 1816 [OFW-4.93 lv1 a:0x30845C-0x30846C]. That slot
//!    [OFW-4.93 lv1 a:0x359920] holds the same class as the slot that
//!    `configure_irq_state_bitmap` checks [OFW-4.93 lv1 a:0x3584B0].
//! 3. It checks that the region holds a page object at +176
//!    [OFW-4.93 lv1 a:0x308470-0x308478].
//! 4. It takes the real address from the page object's page array
//!    [OFW-4.93 lv1 a:0x30847C-0x308498].
//!
//! # The registration
//!
//! The device's doubleword at +48 holds the indicator's real address,
//! and -1 when no indicator is set [OFW-4.93 lv1 a:0x30848C-0x30849C].
//! For a call that passes every check, lv1 does these steps:
//! 1. It keeps the region at +56 and the real address at +48 of the
//!    device [OFW-4.93 lv1 a:0x3084E0-0x3084FC].
//! 2. It adds 1 to the region's count at +192
//!    [OFW-4.93 lv1 a:0x308570-0x308574] [OFW-4.93 lv1 a:0x2DDE8C-0x2DDE94].
//! 3. It returns 0 [OFW-4.93 lv1 a:0x308588].
//!
//! The handler writes no byte at the indicator.
//!
//! # The 4.93 kernel's call
//!
//! The kernel's `sc` [OFW-4.93 lv2_kernel a:0x80000000000A34F4] passes
//! these inputs [OFW-4.93 lv2_kernel a:0x80000000000A34E0-0x80000000000A34F0]:
//! - r3 and r4: the bus id and the device id at +16 and +8 of its
//!   device record;
//! - r5: the logical partition address that the function
//!   [OFW-4.93 lv2_kernel a:0x800000000007014C] gives for the record's
//!   doubleword at +80 [OFW-4.93 lv2_kernel a:0x80000000000A3480-0x80000000000A3494];
//! - r6: 0.
//!
//! For an r3 other than 0, the kernel closes the device with
//! `close_device` and returns the r3 to its caller
//! [OFW-4.93 lv2_kernel a:0x80000000000A34F8-0x80000000000A3504]
//! [OFW-4.93 lv2_kernel a:0x80000000000A3570-0x80000000000A35A0].
//!
//! # `connect_interrupt_event_receive_port`
//!
//! The handler [OFW-4.93 lv1 a:0x305CF0] keeps r3 to r6
//! [OFW-4.93 lv1 a:0x305D04] [OFW-4.93 lv1 a:0x305D20-0x305D28] and does
//! these steps:
//! 1. It refuses a bus id above [`STATUS_INDICATOR_BUS_MAX`] with
//!    [`INTERRUPT_PORT_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x305D88-0x305D8C]
//!    [OFW-4.93 lv1 a:0x305E40-0x305E50].
//! 2. It calls the connect function [OFW-4.93 lv1 a:0x236924] with the
//!    caller's partition and r3 to r6 [OFW-4.93 lv1 a:0x305D90-0x305DB0].
//! 3. It returns that function's r3 [OFW-4.93 lv1 a:0x305DB4]
//!    [OFW-4.93 lv1 a:0x305E1C]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! For device 0 of bus 1, the connect function does these steps. The
//! first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236964-0x23696C]; see below.
//! 2. The device lookup [OFW-4.93 lv1 a:0x236970-0x236980]. For bus 1 it
//!    is the lookup of the access check
//!    [OFW-4.93 lv1 a:0x236648-0x236658], so it finds device 0.
//! 3. An r6 other than 0 gets [`INTERRUPT_PORT_R6_R3`]
//!    [OFW-4.93 lv1 a:0x236A0C-0x236A18] [OFW-4.93 lv1 a:0x236AB4-0x236AE0].
//! 4. The network device's state word, under the lock at +40 of the
//!    global object [OFW-4.93 lv1 a:0x236A1C-0x236A68]. A word other than
//!    2 gives -6, sign-extended [OFW-4.93 lv1 a:0x236A5C]
//!    [OFW-4.93 lv1 a:0x236A6C-0x236AB0]. `open_device` sets the word to
//!    2; see the state word section above.
//! 5. The port connect function [OFW-4.93 lv1 a:0x245F2C], with the
//!    connection object from the TOC slot at r2 - 26624 and r5
//!    [OFW-4.93 lv1 a:0x236BB4-0x236BC0] [OFW-4.93 lv1 a:0x353838]; see
//!    below. Its -6 gives [`INTERRUPT_PORT_NO_PORT_R3`]
//!    [OFW-4.93 lv1 a:0x236BC4-0x236BC8] [OFW-4.93 lv1 a:0x236BEC-0x236BF8].
//!    Its -9 gives [`INTERRUPT_PORT_BUSY_R3`]
//!    [OFW-4.93 lv1 a:0x236BCC-0x236BE4]. Its 0 gives 0
//!    [OFW-4.93 lv1 a:0x236BD0-0x236BD4].
//!
//! # The access check
//!
//! The access check returns a status and a mark. For a mark other than 0,
//! the connect function sets bit (mark - 1) of the high word of the
//! zero-extended status [OFW-4.93 lv1 a:0x236AFC-0x236B1C]. The check's
//! marks are 0, 1 and 2, so the high word is the mark:
//! - Bus 0 gets -6 with the mark 1 [OFW-4.93 lv1 a:0x2363F0-0x236428];
//!   see [`INTERRUPT_PORT_NO_BUS_R3`].
//! - For bus 1, the check looks up the device with the mark 0
//!   [OFW-4.93 lv1 a:0x23648C-0x2364B4].
//! - It then calls the device's entries at +40 and +48
//!   [OFW-4.93 lv1 a:0x2364B8-0x236530]. When either entry returns 0, the
//!   check returns `LV1_WRONG_STATE` (-11) [OFW-4.93 lv1 a:0x236534-0x236538].
//!   Otherwise it returns 0 [OFW-4.93 lv1 a:0x236560].
//!
//! Device 0's entries are these:
//! - The entry at +40 [OFW-4.93 lv1 a:0x35C8A0-0x35C8A4]
//!   [OFW-4.93 lv1 a:0x32D498-0x32D49C] returns the device's open flag at
//!   +108 [OFW-4.93 lv1 a:0x227D58].
//! - The entry at +48 [OFW-4.93 lv1 a:0x35C8A8-0x35C8AC]
//!   [OFW-4.93 lv1 a:0x32D510-0x32D514] returns 1 when the flag is set
//!   and the caller's partition holds the device open
//!   [OFW-4.93 lv1 a:0x228120-0x228144].
//!
//! So a partition that did not open the network device gets
//! [`INTERRUPT_PORT_NOT_OPEN_R3`].
//!
//! # The port connect function
//!
//! The function holds the connection's lock at +8
//! [OFW-4.93 lv1 a:0x245F34-0x245F84] and does these steps:
//! 1. When the connection's port at +32 holds a reference, it returns
//!    `LV1_BUSY` (-9) [OFW-4.93 lv1 a:0x245F88-0x245FB0]. Bits 2 to 31 of
//!    the word at +8 of an object count its references: the reference
//!    function adds 4 [OFW-4.93 lv1 a:0x2BED58-0x2BED6C].
//! 2. It looks up r5 in the calling partition's object table at +88
//!    [OFW-4.93 lv1 a:0x245FFC-0x24610C]. An id that names no object of
//!    the port class [OFW-4.93 lv1 a:0x246114-0x246120]
//!    [OFW-4.93 lv1 a:0x354000] or a port whose byte at +148 is 0
//!    [OFW-4.93 lv1 a:0x2461D8-0x24623C] gets `LV1_NO_ENTRY` (-6)
//!    [OFW-4.93 lv1 a:0x246124-0x246174].
//! 3. It links the port to the connection, which keeps the port at +32
//!    [OFW-4.93 lv1 a:0x246140-0x246150] [OFW-4.93 lv1 a:0x2BEDD4].
//! 4. For a port with a reference [OFW-4.93 lv1 a:0x246154-0x24616C], it
//!    allocates an 88-byte object, puts it on the port's list at +104 and
//!    keeps it at +40 of the connection. It then returns 0
//!    [OFW-4.93 lv1 a:0x24626C-0x246304].
//!
//! `construct_event_receive_port` takes a reference on each port that it
//! puts in the object table [OFW-4.93 lv1 a:0x2E2608-0x2E2618]. An
//! allocation failure in step 4 gives `LV1_RESOURCE_SHORTAGE` (-2)
//! [OFW-4.93 lv1 a:0x246308-0x246310]; no input of the call selects it.
//!
//! # The 4.93 kernel's call of `connect_interrupt_event_receive_port`
//!
//! The kernel's `sc` [OFW-4.93 lv2_kernel a:0x80000000000A2930] passes
//! these inputs [OFW-4.93 lv2_kernel a:0x80000000000A291C-0x80000000000A292C]:
//! - r3 and r4: the bus id and the device id at +16 and +8 of its
//!   device record;
//! - r5: the port id at +72 of the record, which
//!   `construct_event_receive_port` gave it
//!   [OFW-4.93 lv2_kernel a:0x80000000000A28E0-0x80000000000A28FC];
//! - r6: 0.
//!
//! The kernel's function returns 0 whatever r3 the call gets
//! [OFW-4.93 lv2_kernel a:0x80000000000A2934-0x80000000000A2948].
//!
//! # The storage bus's interrupt port
//!
//! For bus 4, the connect function does these steps. The first failure
//! is the answer:
//! 1. The access check. For bus 4 it sets the mark 0 and finds the device
//!    as the lookup does [OFW-4.93 lv1 a:0x236448-0x236450]
//!    [OFW-4.93 lv1 a:0x2364E8-0x236508] [OFW-4.93 lv1 a:0x236554-0x23655C].
//!    A device id above 6, other than -1, finds no device and gets
//!    [`STORAGE_PORT_NO_DEVICE_R3`]. The check then calls the method at
//!    +48 of the device's method table with the caller's partition
//!    [OFW-4.93 lv1 a:0x23650C-0x236530]. A method that returns 0 gives
//!    [`STORAGE_PORT_NOT_OPEN_R3`].
//! 2. The device lookup [OFW-4.93 lv1 a:0x236970-0x236980]. It finds the
//!    device that the access check found
//!    [OFW-4.93 lv1 a:0x2365E0-0x23661C].
//! 3. An r6 other than 0 gets [`STORAGE_PORT_R6_R3`]
//!    [OFW-4.93 lv1 a:0x23698C-0x236998].
//! 4. The storage connect [OFW-4.93 lv1 a:0x255254], with the device and
//!    r5 [OFW-4.93 lv1 a:0x23699C-0x2369BC]. Its -6 gives
//!    [`STORAGE_PORT_NO_PORT_R3`], and its -9 gives
//!    [`STORAGE_PORT_BUSY_R3`]. The connect function returns any other
//!    status zero-extended [OFW-4.93 lv1 a:0x2369CC-0x2369F4].
//!
//! For device -1, the device is the storage bus object. The object's
//! method at +48 [OFW-4.93 lv1 a:0x35DD60-0x35DD64]
//! [OFW-4.93 lv1 a:0x338778-0x33877C] holds the lock at +2536. It returns
//! 1 when a used slot of the object holds the caller's partition
//! [OFW-4.93 lv1 a:0x25477C-0x2547FC]. The slots are the two slots of the
//! open; see the storage bus section of [`super::device_bus`]. So a
//! partition that did not open the storage bus gets
//! [`STORAGE_PORT_NOT_OPEN_R3`].
//!
//! The storage connect holds the same lock
//! [OFW-4.93 lv1 a:0x255280-0x2552A4] and does these steps:
//! 1. It looks up r5 in the calling partition's object table at +88
//!    [OFW-4.93 lv1 a:0x2552A8-0x2553B8]. The port class is the word that
//!    the TOC slot r2 - 23160 holds, 0x329310
//!    [OFW-4.93 lv1 a:0x3545C0-0x3545C4]. Each of these gets -6
//!    [OFW-4.93 lv1 a:0x2553B8-0x2553D0]:
//!    - a free id
//!      [OFW-4.93 lv1 a:0x255300-0x255314] [OFW-4.93 lv1 a:0x2C9DB4-0x2C9E40];
//!    - an empty slot;
//!    - an object of another class.
//! 2. A port whose byte at +148 is 0 gets -6
//!    [OFW-4.93 lv1 a:0x2554B8-0x2554E4].
//! 3. It tests again the flag that is 1 only when the lookup gave a null
//!    object [OFW-4.93 lv1 a:0x255460-0x25547C]
//!    [OFW-4.93 lv1 a:0x2554E8-0x2554EC]. The lookup finds an id in each
//!    segment of the table [OFW-4.93 lv1 a:0x255348-0x2553B4]. So a port
//!    with an id at or above [`STORAGE_PORT_ID_LIMIT`], outside segment 0,
//!    passes.
//! 4. It takes slot 0 when slot 0 is used and holds the caller's
//!    partition. Otherwise it takes slot 1 when slot 1 is used and holds
//!    the caller's partition. Otherwise it returns -6
//!    [OFW-4.93 lv1 a:0x2554F0-0x25551C]
//!    [OFW-4.93 lv1 a:0x2555F8-0x255608].
//! 5. The slot's handle is at +2568 + 64 x slot. When the handle holds a
//!    port with a reference, it returns -9
//!    [OFW-4.93 lv1 a:0x255520-0x255548].
//! 6. When the handle holds a port, it calls the function
//!    [OFW-4.93 lv1 a:0x2BEF4C] with that port and the handle. It then
//!    gives the new port and the handle to the link function
//!    [OFW-4.93 lv1 a:0x25554C-0x255570]. The link function keeps the port
//!    at +16 of the handle [OFW-4.93 lv1 a:0x2BEDD4].
//! 7. It allocates an 88-byte object, puts it on the port's list at +104
//!    and keeps it at +2592 + 64 x slot. It returns 0, also when the
//!    allocation fails [OFW-4.93 lv1 a:0x255574-0x2555F4].
//!
//! The 4.93 kernel's port setup sends the call with these inputs
//! [OFW-4.93 lv2_kernel a:0x80000000000964B8-0x80000000000964CC]:
//! - r3 and r4: the bus id 4 and the device id -1 of its interrupt
//!   object;
//! - r5: the port id that `construct_event_receive_port` gave it;
//! - r6: the interrupt id 0.
//!
//! The setup continues only when r3 is 0
//! [OFW-4.93 lv2_kernel a:0x80000000000964D0-0x80000000000964DC]. The
//! capture [`STORAGE_PORT_CAPTURE`] shows the port with that id at +16 of
//! slot 1's handle after the boot. The capture
//! [`STORAGE_BUS_OPEN_CAPTURE`] shows slot 1 used, with the game OS
//! partition. So the call passed each check and took slot 1.
//!
//! [`STORAGE_PORT_CAPTURE`]: super::event_port::STORAGE_PORT_CAPTURE
//! [`STORAGE_BUS_OPEN_CAPTURE`]: super::device_bus::STORAGE_BUS_OPEN_CAPTURE
//!
//! # A storage device's interrupt port
//!
//! For a device id up to 6, the access check takes the device object D
//! at +3816 + 8 x id of the storage bus object
//! [OFW-4.93 lv1 a:0x2364E8-0x236500]. A null D gets
//! [`STORAGE_PORT_NO_DEVICE_R3`] [OFW-4.93 lv1 a:0x236504-0x236508]. The
//! check then calls the method at +48 of D's method table with the
//! caller's partition [OFW-4.93 lv1 a:0x23650C-0x236530].
//!
//! The capture [`STORAGE_DEVICE_OPEN_CAPTURE`] reads the method table of
//! each of the five devices of the 4.93 bus table; see the storage device
//! section of [`super::device_bus`]. The entry at +48 of each table holds
//! the descriptor 0x338778, the method of the storage bus object:
//! - device 0, table 0x35E518 [OFW-4.93 lv1 a:0x35E548-0x35E54C];
//! - device 1, table 0x35E6E8 [OFW-4.93 lv1 a:0x35E718-0x35E71C];
//! - device 2, table 0x35E370 [OFW-4.93 lv1 a:0x35E3A0-0x35E3A4];
//! - device 3, table 0x35E1C8 [OFW-4.93 lv1 a:0x35E1F8-0x35E1FC];
//! - device 4, table 0x35DFF8 [OFW-4.93 lv1 a:0x35E028-0x35E02C].
//!
//! The method returns 1 when a used slot of D holds the caller's
//! partition [OFW-4.93 lv1 a:0x25477C-0x2547FC]. `open_device` of the
//! device writes that slot; see [`super::device_bus`]. So a partition
//! that did not open the device gets [`STORAGE_PORT_NOT_OPEN_R3`].
//!
//! After the device lookup and the r6 check, the connect function gives D
//! and r5 to the storage connect [OFW-4.93 lv1 a:0x23699C-0x2369BC]. The
//! storage connect does the steps of the storage bus section on D:
//! - It holds the lock at D + 2536 [OFW-4.93 lv1 a:0x255280-0x2552A4].
//! - It checks the port as for the storage bus.
//! - It takes D's slot that holds the caller's partition
//!   [OFW-4.93 lv1 a:0x2554F0-0x25551C] [OFW-4.93 lv1 a:0x2555F8-0x255608].
//! - It returns -9 while the slot's handle at D + 2568 + 64 x slot holds a
//!   port with a reference [OFW-4.93 lv1 a:0x255520-0x255548].
//! - It links the port at +16 of that handle
//!   [OFW-4.93 lv1 a:0x25554C-0x255570] [OFW-4.93 lv1 a:0x2BEDD4].
//!
//! So a storage device gets the answers of the storage bus, in the same
//! order, for D's slot of the caller's partition.
//!
//! The 4.93 kernel's device object makes an interrupt object with the
//! type 2, the interrupt id 0, its bus id and its device id
//! [OFW-4.93 lv2_kernel a:0x80000000000AC154-0x80000000000AC164]. The
//! object keeps them at +16, +24, +80 and +88
//! [OFW-4.93 lv2_kernel a:0x8000000000096334-0x8000000000096350]. The port
//! setup sends the call with the bus id in r3, the device id in r4, the
//! port id in r5 and the interrupt id in r6
//! [OFW-4.93 lv2_kernel a:0x80000000000964B8-0x80000000000964CC].
//!
//! The capture [`STORAGE_DEVICE_PORT_CAPTURE`] shows a port in the game OS
//! slot of each of the five devices after the boot. Each port's id is the
//! id that `construct_event_receive_port` gave the kernel's call for that
//! device. So each of the five calls passed each check.
//!
//! [`STORAGE_DEVICE_OPEN_CAPTURE`]: super::device_bus::STORAGE_DEVICE_OPEN_CAPTURE
//! [`STORAGE_DEVICE_PORT_CAPTURE`]: super::event_port::STORAGE_DEVICE_PORT_CAPTURE
//!
//! # `net_control`
//!
//! The handler [OFW-4.93 lv1 a:0x307D2C] keeps r3 to r8
//! [OFW-4.93 lv1 a:0x307D40] [OFW-4.93 lv1 a:0x307D64-0x307D74] and
//! checks them in this order. The first failure is the answer:
//! 1. a bus id above [`NET_CONTROL_BUS_MAX`] gets
//!    [`NET_CONTROL_BUS_RANGE_R3`];
//! 2. a bus id other than [`NET_CONTROL_BUS`] gets
//!    [`NET_CONTROL_NO_BUS_R3`];
//! 3. a device id other than [`NET_CONTROL_DEVICE`] gets
//!    [`NET_CONTROL_NO_DEVICE_R3`];
//! 4. the network device's state word, see below;
//! 5. the request code, see [`net_control_request_range_r3`];
//! 6. the device's flag byte, see below.
//!
//! For device 0 of bus 1, the handler takes the lock at +40 of the
//! network device's global object and reads the object at +16 of it
//! [OFW-4.93 lv1 a:0x307DEC-0x307E28]. That global object is the object
//! of `net_set_interrupt_status_indicator`: the handler loads it from the
//! same TOC slot at r2 - 1848 [OFW-4.93 lv1 a:0x307DEC]. A state word at
//! +8 of the object at +16 other than 2 gets
//! [`NET_CONTROL_WRONG_STATE_R3`] [OFW-4.93 lv1 a:0x307E2C-0x307E40].
//! `open_device` sets that word to 2; see the state word section above.
//!
//! The handler then calls the request function
//! [OFW-4.93 lv1 a:0x245108] with these arguments
//! [OFW-4.93 lv1 a:0x307F4C-0x307F78]:
//! - the object of the TOC slot at r2 - 1824;
//! - the object at +16 of the global object;
//! - the request code and r6 to r8;
//! - the return slots of r4 and r5, HSPRG0 - 0x6A38 and HSPRG0 - 0x6A30.
//!
//! The handler returns the low word of that function's status,
//! zero-extended [OFW-4.93 lv1 a:0x307F7C] [OFW-4.93 lv1 a:0x307EE0]. The
//! handler writes no return slot itself, and the vector's entry zeroes the
//! slots [OFW-4.93 lv1 a:0x0C08]. So r6 to r10 return 0, and a call that
//! fails before the request function also gets 0 in r4 and r5.
//!
//! # The request function
//!
//! The function reads the byte at +0 of the device's object. A byte of 0
//! gets [`NET_CONTROL_NOT_IMPLEMENTED_R3`]
//! [OFW-4.93 lv1 a:0x245108-0x245118]. Otherwise the function branches to
//! the request switch [OFW-4.93 lv1 a:0x24511C]. The switch zeroes its
//! two outputs [OFW-4.93 lv1 a:0x244ACC-0x244AD0] and stores them in the
//! return slots of r4 and r5 [OFW-4.93 lv1 a:0x244C28-0x244C34].
//!
//! The console capture [`NET_CONTROL_CAPTURE`] reads the byte at +0 as 1
//! and the state word as 2. No lv1 code read stores that byte.
//!
//! For request [`NET_CONTROL_GET_MAC_ADDRESS`], the switch
//! [OFW-4.93 lv1 a:0x244A84] [OFW-4.93 lv1 a:0x244ADC-0x244AE0]
//! [OFW-4.93 lv1 a:0x244B58-0x244B64]
//! [OFW-4.93 lv1 a:0x244C9C-0x244CA0] does these steps:
//! 1. It puts the doubleword at +16 of the device's object in the first
//!    output [OFW-4.93 lv1 a:0x244F9C-0x244FA4].
//! 2. It leaves 0 in the second output.
//! 3. It returns 0 [OFW-4.93 lv1 a:0x244FA0] [OFW-4.93 lv1 a:0x244C2C].
//!
//! That doubleword is the device's MAC address. Linux
//! `drivers/net/ethernet/toshiba/ps3_gelic_net.c`, in
//! `gelic_net_setup_netdev`, sends request 1 and shifts r4 left by 16
//! bits to take the six address bytes. So r4 holds the address in its
//! low [`NET_CONTROL_MAC_ADDRESS_BITS`] bits.
//!
//! # The 4.93 kernel's call of `net_control` request 1
//!
//! The kernel's `sc` [OFW-4.93 lv2_kernel a:0x80000000000A369C] passes
//! these inputs [OFW-4.93 lv2_kernel a:0x80000000000A3680-0x80000000000A3698]:
//! - r3 and r4: the bus id and the device id at +16 and +8 of its
//!   device record;
//! - r5: 1;
//! - r6 to r8: 0.
//!
//! For an r3 other than 0 or an r4 of 0, the kernel's function returns
//! 0x80010005. Otherwise it keeps r4 at +56 of the record
//! [OFW-4.93 lv2_kernel a:0x80000000000A36A0-0x80000000000A36D8].
//!
//! # Request 4, the VLAN id
//!
//! The switch sends request [`NET_CONTROL_GET_VLAN_ID`] to its case at
//! 0x244DD8 [OFW-4.93 lv1 a:0x244ADC-0x244AE0]
//! [OFW-4.93 lv1 a:0x244B58-0x244B74]. The case does these steps:
//! 1. It loads the network device's global object from the TOC slot at
//!    r2 - 24896 [OFW-4.93 lv1 a:0x244DD8-0x244DDC]. That slot holds
//!    0x3682B0 [OFW-4.93 lv1 a:0x353EF8-0x353EFC], the address that the
//!    slot at r2 - 1848 holds.
//! 2. When bit 63 of the doubleword at +32 of the global object is set,
//!    it returns `LV1_NO_ENTRY` (-6) [OFW-4.93 lv1 a:0x244DE0-0x244DEC]
//!    [OFW-4.93 lv1 a:0x245014-0x245018].
//! 3. It fails a selector in r6 above [`NET_CONTROL_VLAN_SELECTOR_MAX`],
//!    unsigned, with `LV1_ILLEGAL_PARAMETER_VALUE` (-17)
//!    [OFW-4.93 lv1 a:0x244DF0-0x244DF4] [OFW-4.93 lv1 a:0x244B24].
//! 4. It branches through the table at 0x244E14 by r6
//!    [OFW-4.93 lv1 a:0x244DF8-0x244E10] [OFW-4.93 lv1 a:0x353F78-0x353F7C];
//!    see [`net_control_vlan_selector`].
//!
//! A case that gives an id stores it in the first output, leaves 0 in the
//! second, and returns 0 [OFW-4.93 lv1 a:0x244C28-0x244C34]. A case that
//! fails goes to the switch's exit and stores no output
//! [OFW-4.93 lv1 a:0x244B28-0x244B54]. The handler keeps the low word of
//! the status [OFW-4.93 lv1 a:0x307F7C], so both failures are
//! zero-extended. The case reads no input other than r6.
//!
//! No lv1 code read stores the doubleword at +32 or the byte at +28 of the
//! global object. The console capture [`NET_CONTROL_VLAN_FIELDS_CAPTURE`]
//! reads both; see [`CONSOLE_VLAN_FIELDS`]. Bit 63 of the doubleword is
//! set there, so the console's lv1 gives -6 for each selector; see
//! [`net_control_vlan_id`].
//!
//! # The 4.93 kernel's calls of `net_control` request 4
//!
//! The kernel's `sc` [OFW-4.93 lv2_kernel a:0x80000000000A2868] passes
//! these inputs [OFW-4.93 lv2_kernel a:0x80000000000A2844-0x80000000000A2864]:
//! - r3 and r4: the bus id and the device id at +16 and +8 of its
//!   device record;
//! - r5: 4;
//! - r6: the function's second argument, the selector;
//! - r7 and r8: 0.
//!
//! For an r3 of 0, the function keeps the low halfword of r4 and returns
//! 0. Otherwise it returns 0x80010005
//! [OFW-4.93 lv2_kernel a:0x80000000000A286C-0x80000000000A2890].
//!
//! The kernel first calls the function with the selector 1
//! [OFW-4.93 lv2_kernel a:0x80000000000A384C-0x80000000000A385C]. When
//! that call fails, it skips the other calls
//! [OFW-4.93 lv2_kernel a:0x80000000000A3860-0x80000000000A386C]. Otherwise
//! it calls the function with the selectors 16 to 20 and then 1 to 5, and
//! it stops at the first call that fails
//! [OFW-4.93 lv2_kernel a:0x80000000000A38F0-0x80000000000A3A54].
//!
//! # `net_set_interrupt_mask`
//!
//! The handler [OFW-4.93 lv1 a:0x307FE0] keeps r3 to r6
//! [OFW-4.93 lv1 a:0x307FEC] [OFW-4.93 lv1 a:0x308000-0x30800C] and
//! checks them in this order. The first failure is the answer:
//! 1. a bus id above [`INTERRUPT_MASK_BUS_MAX`] gets
//!    [`INTERRUPT_MASK_BUS_RANGE_R3`];
//! 2. a bus id other than [`INTERRUPT_MASK_BUS`] gets
//!    [`INTERRUPT_MASK_NO_BUS_R3`];
//! 3. a device id other than [`INTERRUPT_MASK_DEVICE`] gets
//!    [`INTERRUPT_MASK_NO_DEVICE_R3`];
//! 4. the network device's state word, see below;
//! 5. an r6 other than 0 gets [`INTERRUPT_MASK_R6_R3`].
//!
//! For device 0 of bus 1, the handler takes the lock at +40 of the
//! network device's global object and reads the object at +16 of it
//! [OFW-4.93 lv1 a:0x308088-0x3080C4]. The handler loads that global
//! object from the TOC slot at r2 - 1848, as
//! `net_set_interrupt_status_indicator` does [OFW-4.93 lv1 a:0x308088].
//! A state word at +8 of the object at +16 other than 2 gets
//! [`INTERRUPT_MASK_WRONG_STATE_R3`] [OFW-4.93 lv1 a:0x3080C8-0x3080DC].
//! `open_device` sets that word to 2; see the state word section above.
//!
//! A call that passes every check does these steps:
//! 1. It stores r5 at +32 of the object at +16
//!    [OFW-4.93 lv1 a:0x3081CC].
//! 2. It returns 0 [OFW-4.93 lv1 a:0x3081C8] [OFW-4.93 lv1 a:0x30817C].
//!
//! The handler checks no bit of r5, and each call replaces the doubleword
//! at +32. The handler writes no return slot, and the vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! # The 4.93 kernel's calls of `net_set_interrupt_mask`
//!
//! The kernel has six calls. Each call passes the bus id and the device
//! id at +16 and +8 of its device record in r3 and r4, and 0 in r6. The
//! kernel ignores the r3 that each call gets. The calls differ in r5:
//! - [OFW-4.93 lv2_kernel a:0x80000000000A319C] passes 0
//!   [OFW-4.93 lv2_kernel a:0x80000000000A3160-0x80000000000A3198];
//! - [OFW-4.93 lv2_kernel a:0x80000000000A3784] and
//!   [OFW-4.93 lv2_kernel a:0x80000000000A37A4] pass the record's
//!   doubleword at +64 with more bits set
//!   [OFW-4.93 lv2_kernel a:0x80000000000A3764-0x80000000000A37A0];
//! - [OFW-4.93 lv2_kernel a:0x80000000000A4A04] passes the doubleword at
//!   +64 with bits set from r12
//!   [OFW-4.93 lv2_kernel a:0x80000000000A49B8-0x80000000000A4A00];
//! - [OFW-4.93 lv2_kernel a:0x80000000000A568C] passes the doubleword at
//!   +64 OR 0x1002000000006000
//!   [OFW-4.93 lv2_kernel a:0x80000000000A565C-0x80000000000A5688];
//! - [OFW-4.93 lv2_kernel a:0x80000000000A7FF0] passes the doubleword at
//!   +64 OR 0xC0000000
//!   [OFW-4.93 lv2_kernel a:0x80000000000A7FCC-0x80000000000A7FEC].
//!
//! Each call that sets bits stores r5 back at +64 of the record before
//! its `sc`.

use super::status::{
    zero_extended, LV1_DUPLICATE_ENTRY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NOT_IMPLEMENTED,
    LV1_NO_ENTRY, LV1_WRONG_STATE,
};
use super::storage::STORAGE_DEVICE_ID_MAX;
use super::storage_registration::registration;

/// The handler of hypercall 191 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusIndicatorHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `net_set_interrupt_status_indicator` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 191 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const STATUS_INDICATOR_HANDLERS: &[StatusIndicatorHandler] = &[StatusIndicatorHandler {
    firmware: "4.93",
    handler: 0x308204,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn status_indicator_handler(firmware: &str) -> Option<&'static StatusIndicatorHandler> {
    STATUS_INDICATOR_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest bus id that the handler's first check passes, unsigned
/// [OFW-4.93 lv1 a:0x30829C-0x3082A0].
pub const STATUS_INDICATOR_BUS_MAX: u64 = 4;

/// The bus id of the network device [OFW-4.93 lv1 a:0x3082A4-0x3082A8].
pub const STATUS_INDICATOR_BUS: u64 = 1;

/// The device id of the network device on [`STATUS_INDICATOR_BUS`]
/// [OFW-4.93 lv1 a:0x3082AC-0x3082B0].
pub const STATUS_INDICATOR_DEVICE: u64 = 0;

/// The alignment of an indicator address. The handler refuses an r5 with
/// one of its low 3 bits set [OFW-4.93 lv1 a:0x3083FC-0x308404].
pub const STATUS_INDICATOR_ALIGN: u64 = 8;

/// The r3 for a bus id above [`STATUS_INDICATOR_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x3084B0-0x3084BC].
///
/// As with `allocate_memory`, bit 32 marks r3, bit 33 r4, bit 34 r5 and
/// bit 35 r6.
pub const STATUS_INDICATOR_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for a bus id other than [`STATUS_INDICATOR_BUS`] that passes
/// the range check: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x3083E8-0x3083F4].
pub const STATUS_INDICATOR_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 for a device id other than [`STATUS_INDICATOR_DEVICE`] on
/// [`STATUS_INDICATOR_BUS`]: `LV1_NO_ENTRY` (-6), with r4 marked
/// [OFW-4.93 lv1 a:0x308334-0x308340].
pub const STATUS_INDICATOR_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 for a network device whose state word is not 2:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x308300-0x308308].
pub const STATUS_INDICATOR_WRONG_STATE_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The state word that lv1's teardown stores in the network device's
/// object [OFW-4.93 lv1 a:0x23E0F8-0x23E108].
pub const TORN_DOWN_STATE_WORD: u32 = 3;

/// The r3 for an r6 other than 0: `LV1_ILLEGAL_PARAMETER_VALUE` (-17),
/// with r6 marked [OFW-4.93 lv1 a:0x3083CC-0x3083E0].
pub const STATUS_INDICATOR_R6_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 for an indicator address that fails a check:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x308408-0x308414].
///
/// The address fails in these cases:
/// - it is not a multiple of [`STATUS_INDICATOR_ALIGN`];
/// - no memory region of the partition holds it
///   [OFW-4.93 lv1 a:0x308450-0x308458];
/// - the region's class is not the class of the TOC slot at r2 - 1816
///   [OFW-4.93 lv1 a:0x30845C-0x30846C];
/// - the region holds no page object [OFW-4.93 lv1 a:0x308470-0x308478].
pub const STATUS_INDICATOR_ADDRESS_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a call while an indicator is set: `LV1_DUPLICATE_ENTRY`
/// (-7), zero-extended [OFW-4.93 lv1 a:0x30848C-0x3084AC].
pub const STATUS_INDICATOR_SET_R3: u64 = zero_extended(LV1_DUPLICATE_ENTRY);

/// The r3 of the handler's bus and device checks for device `device` of
/// bus `bus`, or `None` when the network device passes them.
///
/// The checks run in this order:
/// 1. a bus id above [`STATUS_INDICATOR_BUS_MAX`] gets
///    [`STATUS_INDICATOR_BUS_RANGE_R3`];
/// 2. a bus id other than [`STATUS_INDICATOR_BUS`] gets
///    [`STATUS_INDICATOR_NO_BUS_R3`];
/// 3. a device id other than [`STATUS_INDICATOR_DEVICE`] gets
///    [`STATUS_INDICATOR_NO_DEVICE_R3`].
pub const fn status_indicator_device_r3(bus: u64, device: u64) -> Option<u64> {
    if bus > STATUS_INDICATOR_BUS_MAX {
        Some(STATUS_INDICATOR_BUS_RANGE_R3)
    } else if bus != STATUS_INDICATOR_BUS {
        Some(STATUS_INDICATOR_NO_BUS_R3)
    } else if device != STATUS_INDICATOR_DEVICE {
        Some(STATUS_INDICATOR_NO_DEVICE_R3)
    } else {
        None
    }
}

/// The handler of hypercall 197 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptPortHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `connect_interrupt_event_receive_port` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 197 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const INTERRUPT_PORT_HANDLERS: &[InterruptPortHandler] = &[InterruptPortHandler {
    firmware: "4.93",
    handler: 0x305CF0,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn interrupt_port_handler(firmware: &str) -> Option<&'static InterruptPortHandler> {
    INTERRUPT_PORT_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 for a bus id above [`STATUS_INDICATOR_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x305E40-0x305E4C].
pub const INTERRUPT_PORT_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236424] [OFW-4.93 lv1 a:0x236AFC-0x236B18].
pub const INTERRUPT_PORT_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the network device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236AFC-0x236B08].
pub const INTERRUPT_PORT_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for an r6 other than 0: `LV1_NO_ENTRY` (-6), with r6 marked
/// [OFW-4.93 lv1 a:0x236AB4-0x236AE0].
pub const INTERRUPT_PORT_R6_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 for a call while a port holds the network device's interrupt:
/// `LV1_BUSY` (-9), with r6 marked [OFW-4.93 lv1 a:0x236BD8-0x236BE4].
pub const INTERRUPT_PORT_BUSY_R3: u64 = 0x0000_0008_FFFF_FFF7;

/// The r3 for a port id that names no port: `LV1_NO_ENTRY` (-6), with r5
/// marked [OFW-4.93 lv1 a:0x236BEC-0x236BF8].
pub const INTERRUPT_PORT_NO_PORT_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 for a device id of bus 4 above 6, other than -1:
/// `LV1_NO_ENTRY` (-6), with r4 marked [OFW-4.93 lv1 a:0x236464-0x236474]
/// [OFW-4.93 lv1 a:0x236AFC-0x236B1C].
pub const STORAGE_PORT_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 when the calling partition did not open the storage bus:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236AFC-0x236B08].
pub const STORAGE_PORT_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for a storage bus call with an r6 other than 0: `LV1_NO_ENTRY`
/// (-6), with r6 marked [OFW-4.93 lv1 a:0x236994-0x236998]
/// [OFW-4.93 lv1 a:0x236AB4-0x236AE0].
pub const STORAGE_PORT_R6_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 when the storage connect returns -6: `LV1_NO_ENTRY`, with r5
/// marked [OFW-4.93 lv1 a:0x2369C0-0x2369C4]
/// [OFW-4.93 lv1 a:0x236BA0-0x236BAC].
pub const STORAGE_PORT_NO_PORT_R3: u64 = 0x0000_0004_FFFF_FFFA;

/// The r3 when the storage connect returns -9: `LV1_BUSY`, with r6 marked
/// [OFW-4.93 lv1 a:0x2369C8-0x2369D0] [OFW-4.93 lv1 a:0x236B8C-0x236B98].
pub const STORAGE_PORT_BUSY_R3: u64 = 0x0000_0008_FFFF_FFF7;

/// The first port id outside segment 0 of the partition's object table.
///
/// Segment 0 holds the ids below 1 << the table's word at +32
/// [OFW-4.93 lv1 a:0x255348-0x255370]. The capture
/// [`NETWORK_PORT_CAPTURE`] reads 6 in that word after the 4.93 boot.
///
/// [`NETWORK_PORT_CAPTURE`]: super::event_port::NETWORK_PORT_CAPTURE
pub const STORAGE_PORT_ID_LIMIT: u64 = 64;

/// Whether CellGov read the path of a storage device's interrupt port for
/// device `device` of bus 4 at the firmware version `firmware`.
///
/// The read covers each device of the bus table of [`registration`] at
/// `firmware`. The capture
/// [`super::device_bus::STORAGE_DEVICE_OPEN_CAPTURE`] reads the method
/// table of each 4.93 device.
pub fn storage_device_port_read(firmware: &str, device: u64) -> bool {
    device <= STORAGE_DEVICE_ID_MAX
        && registration(firmware).is_some_and(|table| table.device(device).is_some())
}

/// The handler of hypercall 194 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetControlHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `net_control` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 194 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const NET_CONTROL_HANDLERS: &[NetControlHandler] = &[NetControlHandler {
    firmware: "4.93",
    handler: 0x307D2C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn net_control_handler(firmware: &str) -> Option<&'static NetControlHandler> {
    NET_CONTROL_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The console capture of the network device's flag byte and state word
/// that `net_control` reads.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const NET_CONTROL_CAPTURE: &str = "tests/micro/lv1_network_control";

/// The largest bus id that the handler's first check passes, unsigned
/// [OFW-4.93 lv1 a:0x307DD4-0x307DD8].
pub const NET_CONTROL_BUS_MAX: u64 = 4;

/// The bus id of the network device [OFW-4.93 lv1 a:0x307DDC-0x307DE0].
pub const NET_CONTROL_BUS: u64 = 1;

/// The device id of the network device on [`NET_CONTROL_BUS`]
/// [OFW-4.93 lv1 a:0x307DE4-0x307DE8].
pub const NET_CONTROL_DEVICE: u64 = 0;

/// The r3 for a bus id above [`NET_CONTROL_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x307FB0-0x307FBC].
pub const NET_CONTROL_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for a bus id other than [`NET_CONTROL_BUS`] that passes the
/// range check: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x307F0C-0x307F18].
pub const NET_CONTROL_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 for a device id other than [`NET_CONTROL_DEVICE`] on
/// [`NET_CONTROL_BUS`]: `LV1_NO_ENTRY` (-6), with r4 marked
/// [OFW-4.93 lv1 a:0x307E6C-0x307E78].
pub const NET_CONTROL_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 for a network device whose state word is not 2:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x307E38-0x307E40].
pub const NET_CONTROL_WRONG_STATE_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The largest request code that the request check passes for a
/// partition without the privilege bit, unsigned
/// [OFW-4.93 lv1 a:0x307F40-0x307F48]. The smallest is 1.
pub const NET_CONTROL_REQUEST_MAX: u64 = 11;

/// The largest request code that the request check passes for a
/// partition with the privilege bit, unsigned
/// [OFW-4.93 lv1 a:0x307F84-0x307F98]. The smallest is 1.
pub const NET_CONTROL_PRIVILEGED_REQUEST_MAX: u64 = 0x8000_0000_0000_0002;

/// The r3 for a request code that fails the request check:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x307F9C-0x307FA8].
pub const NET_CONTROL_REQUEST_RANGE_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The r3 for a device whose byte at +0 is 0: `LV1_NOT_IMPLEMENTED`
/// (-20), zero-extended [OFW-4.93 lv1 a:0x245114]
/// [OFW-4.93 lv1 a:0x307F7C].
pub const NET_CONTROL_NOT_IMPLEMENTED_R3: u64 = zero_extended(LV1_NOT_IMPLEMENTED);

/// The request code that gives the device's MAC address in r4
/// [OFW-4.93 lv1 a:0x244C9C-0x244CA0] [OFW-4.93 lv1 a:0x244F9C-0x244FA4].
pub const NET_CONTROL_GET_MAC_ADDRESS: u64 = 1;

/// The bits of r4 that hold the MAC address that request
/// [`NET_CONTROL_GET_MAC_ADDRESS`] gives: the low 48, the six bytes of
/// the address in order. Linux `drivers/net/ethernet/toshiba/ps3_gelic_net.c`
/// shifts r4 left by 16 bits to take them, in `gelic_net_setup_netdev`.
pub const NET_CONTROL_MAC_ADDRESS_BITS: u32 = 48;

/// The request code that gives a VLAN id in r4 for the selector in r6
/// [OFW-4.93 lv1 a:0x244B58-0x244B74] [OFW-4.93 lv1 a:0x244DD8-0x244E10].
pub const NET_CONTROL_GET_VLAN_ID: u64 = 4;

/// The largest selector that request [`NET_CONTROL_GET_VLAN_ID`] passes,
/// unsigned [OFW-4.93 lv1 a:0x244DF0-0x244DF4].
pub const NET_CONTROL_VLAN_SELECTOR_MAX: u64 = 20;

/// The r3 for a selector that request [`NET_CONTROL_GET_VLAN_ID`] fails:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), zero-extended
/// [OFW-4.93 lv1 a:0x244B24-0x244B2C] [OFW-4.93 lv1 a:0x307F7C].
pub const NET_CONTROL_VLAN_SELECTOR_R3: u64 = zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);

/// The r3 of request [`NET_CONTROL_GET_VLAN_ID`] when bit 63 of the
/// doubleword at +32 of the global object is set: `LV1_NO_ENTRY` (-6),
/// zero-extended [OFW-4.93 lv1 a:0x244DE0-0x244DEC]
/// [OFW-4.93 lv1 a:0x245014-0x245018] [OFW-4.93 lv1 a:0x307F7C].
pub const NET_CONTROL_VLAN_NONE_R3: u64 = zero_extended(LV1_NO_ENTRY);

/// The bit of the doubleword at +32 of the global object that makes
/// request [`NET_CONTROL_GET_VLAN_ID`] fail with
/// [`NET_CONTROL_VLAN_NONE_R3`]: bit 63, the low bit
/// [OFW-4.93 lv1 a:0x244DE0-0x244DEC].
pub const NET_CONTROL_VLAN_NONE_BIT: u64 = 1;

/// The id that selector 5 gives when the byte at +28 of the global
/// object is 0 [OFW-4.93 lv1 a:0x244EC8-0x244ED0]
/// [OFW-4.93 lv1 a:0x244E88-0x244E8C].
pub const NET_CONTROL_VLAN_FLAG_CLEAR_ID: u64 = 294;

/// The id that selector 5 gives when the byte at +28 of the global
/// object is not 0 [OFW-4.93 lv1 a:0x244ED4-0x244EE0].
pub const NET_CONTROL_VLAN_FLAG_SET_ID: u64 = 296;

/// The console capture of the two fields of the network device's global
/// object that request [`NET_CONTROL_GET_VLAN_ID`] reads.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const NET_CONTROL_VLAN_FIELDS_CAPTURE: &str = "tests/micro/lv1_network_vlan_fields";

/// The offset in the global object of [`VlanFields::capability`]
/// [OFW-4.93 lv1 a:0x244DE0].
pub const NET_CONTROL_VLAN_CAPABILITY_OFFSET: u64 = 32;

/// The offset in the global object of [`VlanFields::flag`]
/// [OFW-4.93 lv1 a:0x244EC8].
pub const NET_CONTROL_VLAN_FLAG_OFFSET: u64 = 28;

/// The fields of the network device's global object that request
/// [`NET_CONTROL_GET_VLAN_ID`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VlanFields {
    /// The doubleword at [`NET_CONTROL_VLAN_CAPABILITY_OFFSET`].
    pub capability: u64,
    /// The byte at [`NET_CONTROL_VLAN_FLAG_OFFSET`].
    pub flag: u8,
}

/// The fields that the reference console's lv1 holds after the boot to
/// the XMB, as the capture [`NET_CONTROL_VLAN_FIELDS_CAPTURE`] reads them.
///
/// No lv1 code read stores either field. The capture does not show the
/// values at the time of the kernel's first request 4.
pub const CONSOLE_VLAN_FIELDS: VlanFields = VlanFields {
    capability: 1,
    flag: 0,
};

/// The answer of request [`NET_CONTROL_GET_VLAN_ID`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlanAnswer {
    /// r3 = 0, with this id in r4 and 0 in r5
    /// [OFW-4.93 lv1 a:0x244C28-0x244C34].
    Id(u64),
    /// This r3. The case stores no output
    /// [OFW-4.93 lv1 a:0x244B28-0x244B54].
    Failed(u64),
}

/// The answer of request [`NET_CONTROL_GET_VLAN_ID`] for the selector
/// `selector` when the global object holds `fields`.
///
/// The case does these steps in this order:
/// 1. When `fields.capability` has [`NET_CONTROL_VLAN_NONE_BIT`] set, it
///    fails with [`NET_CONTROL_VLAN_NONE_R3`]. No other bit changes the
///    answer.
/// 2. It fails a selector that [`net_control_vlan_selector`] gives as
///    [`VlanSelector::Illegal`] with [`NET_CONTROL_VLAN_SELECTOR_R3`].
/// 3. For [`VlanSelector::Flagged`], it gives
///    [`NET_CONTROL_VLAN_FLAG_CLEAR_ID`] when `fields.flag` is 0, and
///    [`NET_CONTROL_VLAN_FLAG_SET_ID`] otherwise.
pub const fn net_control_vlan_id(selector: u64, fields: VlanFields) -> VlanAnswer {
    if fields.capability & NET_CONTROL_VLAN_NONE_BIT != 0 {
        return VlanAnswer::Failed(NET_CONTROL_VLAN_NONE_R3);
    }
    match net_control_vlan_selector(selector) {
        VlanSelector::Fixed(id) => VlanAnswer::Id(id),
        VlanSelector::Flagged if fields.flag == 0 => VlanAnswer::Id(NET_CONTROL_VLAN_FLAG_CLEAR_ID),
        VlanSelector::Flagged => VlanAnswer::Id(NET_CONTROL_VLAN_FLAG_SET_ID),
        VlanSelector::Illegal => VlanAnswer::Failed(NET_CONTROL_VLAN_SELECTOR_R3),
    }
}

/// What the switch of request [`NET_CONTROL_GET_VLAN_ID`] gives for one
/// selector when bit 63 of the capability doubleword is clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlanSelector {
    /// The case gives this id in r4.
    Fixed(u64),
    /// The case gives 294 when the byte at +28 of the global object is 0,
    /// and 296 otherwise [OFW-4.93 lv1 a:0x244EC8-0x244EE0]
    /// [OFW-4.93 lv1 a:0x244E88-0x244E8C].
    Flagged,
    /// The case fails the selector with [`NET_CONTROL_VLAN_SELECTOR_R3`].
    Illegal,
}

/// The switch's case for the selector `selector` of request
/// [`NET_CONTROL_GET_VLAN_ID`].
///
/// The table at 0x244E14 gives these cases
/// [OFW-4.93 lv1 a:0x244E14-0x244E64]:
/// - 1 gives 292 [OFW-4.93 lv1 a:0x244EA8-0x244EB4];
/// - 2 gives 294 [OFW-4.93 lv1 a:0x244E88-0x244E94];
/// - 3 gives 295 [OFW-4.93 lv1 a:0x244E98-0x244EA4];
/// - 4 gives 291 [OFW-4.93 lv1 a:0x244EB8-0x244EC4];
/// - 5 is [`VlanSelector::Flagged`];
/// - 16 gives 288 [OFW-4.93 lv1 a:0x244EE4-0x244EF0];
/// - 17 gives 289 [OFW-4.93 lv1 a:0x244EF4-0x244F00];
/// - 18 gives 290 [OFW-4.93 lv1 a:0x244F04-0x244F10];
/// - 19 gives 261 [OFW-4.93 lv1 a:0x244F14-0x244F20];
/// - 20 gives 256 [OFW-4.93 lv1 a:0x244F24-0x244F30].
///
/// The entries for 0 and for 6 to 15 lead to the failure at 0x244B24, as
/// a selector above [`NET_CONTROL_VLAN_SELECTOR_MAX`] does.
pub const fn net_control_vlan_selector(selector: u64) -> VlanSelector {
    match selector {
        1 => VlanSelector::Fixed(292),
        2 => VlanSelector::Fixed(294),
        3 => VlanSelector::Fixed(295),
        4 => VlanSelector::Fixed(291),
        5 => VlanSelector::Flagged,
        16 => VlanSelector::Fixed(288),
        17 => VlanSelector::Fixed(289),
        18 => VlanSelector::Fixed(290),
        19 => VlanSelector::Fixed(261),
        20 => VlanSelector::Fixed(256),
        _ => VlanSelector::Illegal,
    }
}

/// The r3 of the handler's bus and device checks for device `device` of
/// bus `bus`, or `None` when the network device passes them.
///
/// The checks run in this order:
/// 1. a bus id above [`NET_CONTROL_BUS_MAX`] gets
///    [`NET_CONTROL_BUS_RANGE_R3`];
/// 2. a bus id other than [`NET_CONTROL_BUS`] gets
///    [`NET_CONTROL_NO_BUS_R3`];
/// 3. a device id other than [`NET_CONTROL_DEVICE`] gets
///    [`NET_CONTROL_NO_DEVICE_R3`].
pub const fn net_control_device_r3(bus: u64, device: u64) -> Option<u64> {
    if bus > NET_CONTROL_BUS_MAX {
        Some(NET_CONTROL_BUS_RANGE_R3)
    } else if bus != NET_CONTROL_BUS {
        Some(NET_CONTROL_NO_BUS_R3)
    } else if device != NET_CONTROL_DEVICE {
        Some(NET_CONTROL_NO_DEVICE_R3)
    } else {
        None
    }
}

/// The r3 of the handler's request check for the request code `request`,
/// or `None` when the code passes it.
///
/// The check passes a code from 1 to [`NET_CONTROL_REQUEST_MAX`]. When
/// `privileged` is true, it passes a code from 1 to
/// [`NET_CONTROL_PRIVILEGED_REQUEST_MAX`] instead. The handler takes
/// `privileged` from one bit of the mask that the function
/// [OFW-4.93 lv1 a:0x309664] gives. The index of the bit is the
/// doubleword at +80 of the object at HSPRG0 - 26600
/// [OFW-4.93 lv1 a:0x307F20-0x307F3C] [OFW-4.93 lv1 a:0x309844-0x309860].
pub const fn net_control_request_range_r3(request: u64, privileged: bool) -> Option<u64> {
    let max = if privileged {
        NET_CONTROL_PRIVILEGED_REQUEST_MAX
    } else {
        NET_CONTROL_REQUEST_MAX
    };
    if request.wrapping_sub(1) > max - 1 {
        Some(NET_CONTROL_REQUEST_RANGE_R3)
    } else {
        None
    }
}

/// The handler of hypercall 193 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptMaskHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `net_set_interrupt_mask` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 193 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const INTERRUPT_MASK_HANDLERS: &[InterruptMaskHandler] = &[InterruptMaskHandler {
    firmware: "4.93",
    handler: 0x307FE0,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn interrupt_mask_handler(firmware: &str) -> Option<&'static InterruptMaskHandler> {
    INTERRUPT_MASK_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest bus id that the handler's first check passes, unsigned
/// [OFW-4.93 lv1 a:0x308070-0x308074].
pub const INTERRUPT_MASK_BUS_MAX: u64 = 4;

/// The bus id of the network device [OFW-4.93 lv1 a:0x308078-0x30807C].
pub const INTERRUPT_MASK_BUS: u64 = 1;

/// The device id of the network device on [`INTERRUPT_MASK_BUS`]
/// [OFW-4.93 lv1 a:0x308080-0x308084].
pub const INTERRUPT_MASK_DEVICE: u64 = 0;

/// The r3 for a bus id above [`INTERRUPT_MASK_BUS_MAX`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x3081D4-0x3081E0].
pub const INTERRUPT_MASK_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 for a bus id other than [`INTERRUPT_MASK_BUS`] that passes the
/// range check: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x3081B4-0x3081C0].
pub const INTERRUPT_MASK_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 for a device id other than [`INTERRUPT_MASK_DEVICE`] on
/// [`INTERRUPT_MASK_BUS`]: `LV1_NO_ENTRY` (-6), with r4 marked
/// [OFW-4.93 lv1 a:0x308108-0x308114].
pub const INTERRUPT_MASK_NO_DEVICE_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 for a network device whose state word is not 2:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x3080D4-0x3080DC].
pub const INTERRUPT_MASK_WRONG_STATE_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 for an r6 other than 0: `LV1_ILLEGAL_PARAMETER_VALUE` (-17),
/// with r6 marked [OFW-4.93 lv1 a:0x308198-0x3081AC].
pub const INTERRUPT_MASK_R6_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 of the handler's bus and device checks for device `device` of
/// bus `bus`, or `None` when the network device passes them.
///
/// The checks run in this order:
/// 1. a bus id above [`INTERRUPT_MASK_BUS_MAX`] gets
///    [`INTERRUPT_MASK_BUS_RANGE_R3`];
/// 2. a bus id other than [`INTERRUPT_MASK_BUS`] gets
///    [`INTERRUPT_MASK_NO_BUS_R3`];
/// 3. a device id other than [`INTERRUPT_MASK_DEVICE`] gets
///    [`INTERRUPT_MASK_NO_DEVICE_R3`].
pub const fn interrupt_mask_device_r3(bus: u64, device: u64) -> Option<u64> {
    if bus > INTERRUPT_MASK_BUS_MAX {
        Some(INTERRUPT_MASK_BUS_RANGE_R3)
    } else if bus != INTERRUPT_MASK_BUS {
        Some(INTERRUPT_MASK_NO_BUS_R3)
    } else if device != INTERRUPT_MASK_DEVICE {
        Some(INTERRUPT_MASK_NO_DEVICE_R3)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/network_status_indicator_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/network_interrupt_mask_tests.rs"]
mod interrupt_mask_tests;

#[cfg(test)]
#[path = "tests/network_interrupt_port_tests.rs"]
mod interrupt_port_tests;

#[cfg(test)]
#[path = "tests/network_storage_device_port_tests.rs"]
mod storage_device_port_tests;

#[cfg(test)]
#[path = "tests/network_control_tests.rs"]
mod control_tests;

#[cfg(test)]
#[path = "tests/network_control_console_tests.rs"]
mod control_console_tests;

#[cfg(test)]
#[path = "tests/network_control_vlan_tests.rs"]
mod control_vlan_tests;

#[cfg(test)]
#[path = "tests/network_control_vlan_console_tests.rs"]
mod control_vlan_console_tests;

#[cfg(test)]
#[path = "tests/network_control_vlan_fields_tests.rs"]
mod control_vlan_fields_tests;

#[cfg(test)]
#[path = "tests/network_control_vlan_fields_console_tests.rs"]
mod control_vlan_fields_console_tests;
