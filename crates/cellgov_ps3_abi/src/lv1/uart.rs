//! The virtual UART of a partition:
//! - the handler of `configure_virtual_uart_irq` (hypercall 166);
//! - its argument checks and the r3 of each;
//! - the irq outlet that lv1 constructs, and the bitmap that it fills;
//! - the outlet id of the GameOS kernel's call;
//! - the handler of `get_virtual_uart_param` (hypercall 165), its
//!   checks, and the buffer sizes of the GameOS kernel's ports;
//! - the handler of `set_virtual_uart_param` (hypercall 164), its checks,
//!   and the interrupt status that lv1 computes after each call;
//! - the handler of `read_virtual_uart` (hypercall 162), its checks, and
//!   the writers of a port's receive buffer;
//! - the handler of `write_virtual_uart` (hypercall 163), its checks, and
//!   the bytes that it writes to the peer's receive buffer;
//! - the ports of lv1's system manager and dispatcher manager, and the
//!   bytes that their receive buffers hold at the GameOS kernel's first
//!   read;
//! - the request header that the dispatcher manager reads, the GameOS
//!   kernel's first request, and the reply to it;
//! - the request header that the system manager reads, the bytes that it
//!   reads before a request handler runs, and the GameOS kernel's first
//!   request to it.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2F2810] reads r3 and no other register
//! [OFW-4.93 lv1 a:0x2F2824]. r3 is the logical partition address of a
//! 32-byte bitmap. The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x2F2A48]. Only a call that succeeds writes the return
//! slot of r4; see [`super::event_port`] for that slot. The vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0, and r4
//! returns 0 for a call that fails.
//!
//! # The checks
//!
//! The handler checks r3 in this order, and the first failure is the
//! answer:
//! 1. r3 is not above [`VIRTUAL_UART_IRQ_ADDRESS_MAX`].
//! 2. A memory region holds r3. The handler looks up the region with the
//!    function [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F28BC-0x2F28F8],
//!    as `configure_irq_state_bitmap` does; see [`super::interrupt`].
//! 3. The region's class at +72 is the class in the TOC slot at r2 - 5872
//!    [OFW-4.93 lv1 a:0x2F28FC-0x2F2910]. That slot and the slot that
//!    `configure_irq_state_bitmap` checks, r2 - 7048, hold the same class
//!    [OFW-4.93 lv1 a:0x358948] [OFW-4.93 lv1 a:0x3584B0].
//! 4. The region holds [`VIRTUAL_UART_IRQ_BITMAP_BYTES`] bytes from r3.
//! 5. The bitmap and its next byte are in one page of the region.
//! 6. The region has a page array at +176 [OFW-4.93 lv1 a:0x2F2948-0x2F2950].
//!
//! See [`CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3`] for the r3 of each
//! check.
//!
//! # The configure step
//!
//! The handler translates r3 to a real address through the region's page
//! array [OFW-4.93 lv1 a:0x2F2954-0x2F2984]. It then gives the function
//! [OFW-4.93 lv1 a:0x2F13FC] the partition's virtual UART interrupt
//! object, at +0x120 of the partition, and the real address
//! [OFW-4.93 lv1 a:0x2F2958-0x2F298C]. That function does these steps in
//! this order:
//! 1. It returns 5 when the object holds an outlet at +16
//!    [OFW-4.93 lv1 a:0x2F1474-0x2F1480]. The handler maps 5 to
//!    [`CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3`].
//! 2. It allocates a 152-byte object and constructs it with the function
//!    [OFW-4.93 lv1 a:0x2DF6B8] [OFW-4.93 lv1 a:0x2F1484-0x2F14B0]. That
//!    function has the instructions and the TOC slots of the event receive
//!    port constructor [OFW-4.93 lv1 a:0x2DF60C-0x2DF6B4]
//!    [OFW-4.93 lv1 a:0x2DF6B8-0x2DF760]. So the outlet takes the first
//!    free id of the partition's object table, and has the port class and
//!    no connection; see [`super::event_port`].
//! 3. It checks that the region is live and is the caller's
//!    [OFW-4.93 lv1 a:0x2F1528-0x2F1568] [OFW-4.93 lv1 a:0x2F1790-0x2F17DC].
//! 4. It keeps the real address at +48, the region at +24 and the outlet
//!    at +16 of the object
//!    [OFW-4.93 lv1 a:0x2F15C4-0x2F15CC] [OFW-4.93 lv1 a:0x2F15E8-0x2F15EC].
//! 5. It copies the object's 32-byte shadow at +56 to the bitmap
//!    [OFW-4.93 lv1 a:0x2F15FC-0x2F1620].
//! 6. It puts the outlet in the slot of the object table that the
//!    outlet's id at +72 names [OFW-4.93 lv1 a:0x2F1624-0x2F16F0].
//! 7. It gives the outlet to the event function [OFW-4.93 lv1 a:0x2DF808]
//!    when the count of set shadow bits at +2200 is above 0
//!    [OFW-4.93 lv1 a:0x2F16F4-0x2F1708].
//!
//! The handler then stores the outlet's id in the return slot of r4, and
//! returns 0 [OFW-4.93 lv1 a:0x2F2A6C-0x2F2A8C].
//!
//! The outlet passes each check of `connect_irq_plug_ext`, as an irq
//! outlet of a logical SPE does; see [`super::spe`]. The constructor
//! stores the word in the TOC slot at r2 - 6032 at +136 of the outlet
//! [OFW-4.93 lv1 a:0x2F14AC] [OFW-4.93 lv1 a:0x2DF710]. That word
//! [OFW-4.93 lv1 a:0x3588A8] is not the word of a port
//! [OFW-4.93 lv1 a:0x358160]. So `set_spe_transition_notifier` refuses the
//! outlet, as it refuses an outlet of a logical SPE.
//!
//! # The shadow
//!
//! The object's init function [OFW-4.93 lv1 a:0x2F1258] sets these
//! fields:
//! - +16, the outlet, is 0 [OFW-4.93 lv1 a:0x2F12AC].
//! - +48 holds the address of the object's own doublewords at +88
//!   [OFW-4.93 lv1 a:0x2F1270] [OFW-4.93 lv1 a:0x2F12BC].
//! - +56 to +87, the shadow, are 0 [OFW-4.93 lv1 a:0x2F130C-0x2F1318].
//! - +2200, the count of set shadow bits, is 0 [OFW-4.93 lv1 a:0x2F1308].
//!
//! The raise step sets a shadow bit, adds 1 to the count, and stores the
//! shadow doubleword at the address that +48 holds
//! [OFW-4.93 lv1 a:0x2F225C-0x2F2278]. It does this only when the
//! endpoint's interrupt mask at +152 holds a bit of the event
//! [OFW-4.93 lv1 a:0x2F221C-0x2F2238]. The endpoint constructor sets the
//! mask to 0 [OFW-4.93 lv1 a:0x2F18DC]. The mask setter
//! [OFW-4.93 lv1 a:0x2F3F40-0x2F3F8C] is a step of the worker
//! [OFW-4.93 lv1 a:0x2F3C0C] of `set_virtual_uart_param` (hypercall 164),
//! which its handler calls [OFW-4.93 lv1 a:0x2F419C]. So the shadow and
//! the count stay 0 until the partition sets an endpoint's mask to a
//! value other than 0; see the section on the interrupt status.
//!
//! # The parameters of a port
//!
//! The handler of hypercall 165 [OFW-4.93 lv1 a:0x2F3A68] keeps r3 and r4
//! [OFW-4.93 lv1 a:0x2F3A78] [OFW-4.93 lv1 a:0x2F3A90]. It zeroes a stack
//! slot, and gives r3, r4 and the slot to the worker
//! [OFW-4.93 lv1 a:0x2F3AF0-0x2F3B04]. It then stores the slot in the
//! return slot of r4 [OFW-4.93 lv1 a:0x2F3B08-0x2F3B18], and returns the
//! worker's r3 [OFW-4.93 lv1 a:0x2F3B7C]. The worker writes the slot
//! only when it returns 0, and the vector's entry zeroes the return slots
//! [OFW-4.93 lv1 a:0x0C08]. So r4 returns 0 for a call that fails, and r5
//! to r10 return 0.
//!
//! The worker [OFW-4.93 lv1 a:0x2F34E0] does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
//! 2. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object, at +0x120 of the partition
//!    [OFW-4.93 lv1 a:0x2F34FC-0x2F3580]. It refuses a slot with no
//!    endpoint [OFW-4.93 lv1 a:0x2F3584-0x2F35C0].
//! 3. It refuses r4 above [`VIRTUAL_UART_PARAM_MAX`]
//!    [OFW-4.93 lv1 a:0x2F35C4-0x2F35D8].
//! 4. It reads the parameter through a table of eight entries
//!    [OFW-4.93 lv1 a:0x2F35F0-0x2F3628], stores it in the slot, and
//!    returns 0. The entry for 7 computes the port's interrupt status;
//!    see [`VIRTUAL_UART_PARAM_INTERRUPT_STATUS`].
//!
//! The endpoint's init function stores its buffer size at +112
//! [OFW-4.93 lv1 a:0x2EF650] [OFW-4.93 lv1 a:0x2EF68C].
//! `set_virtual_uart_param` sets only the parameters 0 to 2 and refuses
//! the others [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08]
//! [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04]. So no hypercall changes a buffer
//! size. lv1 makes the endpoints when it makes the partition, so no image
//! read gives their sizes; see [`GAME_OS_VIRTUAL_UART_PORTS`].
//!
//! lv1's own code can change the two sizes while the partition runs:
//! - The endpoint holds its peer in a weak reference at +56, whose target
//!   is at +72 [OFW-4.93 lv1 a:0x2EF734-0x2EF744]. A function links two
//!   endpoints as peers [OFW-4.93 lv1 a:0x2EFE6C-0x2EFEC0], and another
//!   unlinks them [OFW-4.93 lv1 a:0x2EFB50-0x2EFB60]. lv1's service code
//!   calls the link function [OFW-4.93 lv1 a:0x29A218]
//!   [OFW-4.93 lv1 a:0x2AA320] [OFW-4.93 lv1 a:0x2AA420]
//!   [OFW-4.93 lv1 a:0x2F5904].
//! - The word at +8 is the reference count, in steps of 4
//!   [OFW-4.93 lv1 a:0x2BED58-0x2BED6C] [OFW-4.93 lv1 a:0x2BEE78-0x2BEE8C].
//!   So parameter 5 gives 0 once the peer has no reference.
//! - The port function puts a new endpoint in the port's slot and
//!   releases the old one [OFW-4.93 lv1 a:0x2F1A0C-0x2F1A1C]. lv1's service
//!   code calls it [OFW-4.93 lv1 a:0x29A390] [OFW-4.93 lv1 a:0x2A99FC].
//!
//! No image read shows when that code runs. So the capture gives the sizes
//! only at the kernel's own calls; see [`GAME_OS_VIRTUAL_UART_PARAM_CALLS`].
//!
//! # Setting a parameter of a port
//!
//! The handler of hypercall 164 [OFW-4.93 lv1 a:0x2F4100] gives r3, r4
//! and r5 to the worker [OFW-4.93 lv1 a:0x2F4190-0x2F419C]. It returns
//! the worker's r3 [OFW-4.93 lv1 a:0x2F41A4] [OFW-4.93 lv1 a:0x2F4208],
//! and writes no return slot. The vector's entry zeroes the return slots
//! [OFW-4.93 lv1 a:0x0C08], so r4 to r10 return 0.
//!
//! The worker [OFW-4.93 lv1 a:0x2F3C0C] does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F3C0C] [OFW-4.93 lv1 a:0x2F3C30].
//! 2. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object [OFW-4.93 lv1 a:0x2F3C34-0x2F3CB8].
//!    It refuses a slot with no endpoint [OFW-4.93 lv1 a:0x2F3CF4-0x2F3CF8].
//! 3. It refuses r4 above [`VIRTUAL_UART_SET_PARAM_MAX`]
//!    [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04].
//! 4. For the mask, it refuses r5 above [`VIRTUAL_UART_INTERRUPT_MASK_MAX`].
//! 5. It stores r5 in the endpoint's field of r4; see
//!    [`VIRTUAL_UART_PARAM_TX_TRIGGER`] for the fields.
//! 6. It calls each listener in the list at +48 of the endpoint
//!    [OFW-4.93 lv1 a:0x2F3D70-0x2F3DC4], and returns 0
//!    [OFW-4.93 lv1 a:0x2F3DC8-0x2F3DCC].
//!
//! The endpoint constructor sets the three fields to 0
//! [OFW-4.93 lv1 a:0x2F1864] [OFW-4.93 lv1 a:0x2F18D4-0x2F18DC]. The
//! image read has no other store to the three fields of an endpoint.
//! `get_virtual_uart_param` reads them for the parameters 0 to 2
//! [OFW-4.93 lv1 a:0x2F3670] [OFW-4.93 lv1 a:0x2F36F4]
//! [OFW-4.93 lv1 a:0x2F3748]. The port function can put a new endpoint in
//! the port's slot, as the section on the parameters of a port gives. So
//! the three values of a port can return to 0 at a time that no image
//! read shows.
//!
//! # The interrupt status
//!
//! The port function adds a listener to the endpoint that it puts in a
//! port's slot [OFW-4.93 lv1 a:0x2F1A50-0x2F1A70]. The listener holds the
//! virtual UART object and the status function
//! [OFW-4.93 lv1 a:0x2F0F18-0x2F0F94] [OFW-4.93 lv1 a:0x3588C0]
//! [OFW-4.93 lv1 a:0x349908]. The method that the worker calls
//! [OFW-4.93 lv1 a:0x3625F0-0x362604] [OFW-4.93 lv1 a:0x3495A8] calls the
//! status function with the object and the endpoint
//! [OFW-4.93 lv1 a:0x2F0AF4-0x2F0B38].
//!
//! The list at +48 of an endpoint holds only this listener:
//! - The port function's call is the only call of the listener's builder
//!   in the image read, and no word of the image holds the builder's
//!   descriptor [OFW-4.93 lv1 a:0x349728].
//! - The builder claims the slot at +160 of the endpoint. When the slot
//!   holds a listener, it removes the new listener from the list again
//!   [OFW-4.93 lv1 a:0x2F0FBC-0x2F1058].
//! - The other builders of the same form add their listener to an object
//!   that they make. That object is no endpoint
//!   [OFW-4.93 lv1 a:0x209E5C-0x209ED4] [OFW-4.93 lv1 a:0x22E0B4-0x22E12C]
//!   [OFW-4.93 lv1 a:0x2F55D8-0x2F5668].
//!
//! The status function [OFW-4.93 lv1 a:0x2F20E4] acts only when the
//! port's slot holds the endpoint [OFW-4.93 lv1 a:0x2F2128-0x2F2140]. It
//! sets these bits of the status:
//! - 1 when the endpoint has a peer with a reference, and the doubleword
//!   at +120 of the peer is not above the transmit trigger
//!   [OFW-4.93 lv1 a:0x2F21E0-0x2F2204] [OFW-4.93 lv1 a:0x2F2380-0x2F2384];
//! - 2 when the buffer size less the doubleword at +120 of the endpoint
//!   is not above the receive trigger
//!   [OFW-4.93 lv1 a:0x2F2208-0x2F2210] [OFW-4.93 lv1 a:0x2F233C-0x2F2348];
//! - 4 when the endpoint has no peer with a reference
//!   [OFW-4.93 lv1 a:0x2F21B0-0x2F21C8] [OFW-4.93 lv1 a:0x2F21E8]
//!   [OFW-4.93 lv1 a:0x2F23A4];
//! - 8 when the endpoint has a peer with a reference
//!   [OFW-4.93 lv1 a:0x2F21F4-0x2F21FC]. No mask that lv1 accepts holds
//!   this bit.
//!
//! The function then does one of these steps:
//! - When the byte at +128 of the endpoint is 0, and the status and the
//!   mask share a bit, it sets the port's shadow bit, as the section on
//!   the shadow gives [OFW-4.93 lv1 a:0x2F217C-0x2F2184]
//!   [OFW-4.93 lv1 a:0x2F221C-0x2F2278]. It gives the outlet to the event
//!   function when the count becomes 1
//!   [OFW-4.93 lv1 a:0x2F227C-0x2F2280] [OFW-4.93 lv1 a:0x2F2394-0x2F239C].
//! - Otherwise, when the port's shadow bit is set, it clears the bit
//!   [OFW-4.93 lv1 a:0x2F22DC-0x2F2334].
//!
//! So a call that leaves the mask at 0 sets no shadow bit, and changes no
//! byte while the shadow is 0. A call that leaves a mask other than 0 can
//! raise the port's interrupt. Whether it does depends on the peer and on
//! the doublewords at +120, which change with the port's traffic;
//! [`virtual_uart_status`] computes the status from them. The kernel sets
//! such a mask on each of its ports; see [`VIRTUAL_UART_PORT_MASK_CAPTURE`].
//!
//! # Reading a port
//!
//! The handler of hypercall 162 [OFW-4.93 lv1 a:0x2F429C] keeps r3, r4
//! and r5 [OFW-4.93 lv1 a:0x2F42B0] [OFW-4.93 lv1 a:0x2F42D8-0x2F42DC].
//! It does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F433C-0x2F4340].
//! 2. It refuses r4 above [`VIRTUAL_UART_READ_ADDRESS_MAX`].
//! 3. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object [OFW-4.93 lv1 a:0x2F4354-0x2F43D4].
//!    It refuses a slot with no endpoint [OFW-4.93 lv1 a:0x2F4410-0x2F4414].
//! 4. It looks up the memory region of r4 with the function
//!    [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F4418-0x2F4444]. It
//!    checks the region's class against the TOC slot at r2 - 5872, as
//!    `configure_virtual_uart_irq` does [OFW-4.93 lv1 a:0x2F4448-0x2F4458].
//! 5. It refuses a region with fewer than r5 bytes from r4 to its end
//!    [OFW-4.93 lv1 a:0x2F445C-0x2F446C], and a region with no page array
//!    at +176 [OFW-4.93 lv1 a:0x2F4470-0x2F4478].
//! 6. For each page of the buffer, it gives the endpoint, the page's real
//!    address and the bytes left in the page to the read function
//!    [OFW-4.93 lv1 a:0x2F447C-0x2F44F4]. It stops after a read of 0
//!    bytes, or when no byte is left [OFW-4.93 lv1 a:0x2F4500-0x2F4528].
//! 7. It stores the sum of the bytes in the return slot of r4, and returns
//!    0 [OFW-4.93 lv1 a:0x2F452C-0x2F4538] [OFW-4.93 lv1 a:0x2F45A0].
//!
//! A refusal skips that store [OFW-4.93 lv1 a:0x2F4604-0x2F4614], and the
//! vector's entry zeroes the return slots [OFW-4.93 lv1 a:0x0C08]. So r4
//! returns 0 for a call that fails, and r5 to r10 return 0. See
//! [`READ_VIRTUAL_UART_NO_PORT_R3`] for the r3 of each check.
//!
//! The read function [OFW-4.93 lv1 a:0x2F06C0] reads the endpoint's
//! receive buffer:
//! - It returns 7 when the byte at +128 of the endpoint is not 0
//!   [OFW-4.93 lv1 a:0x2F0740-0x2F074C].
//! - It copies the lesser of the bytes asked for and the doubleword at
//!   +120, the count of bytes that the buffer holds
//!   [OFW-4.93 lv1 a:0x2F0750-0x2F0770]. It copies from the read pointer
//!   at +96, which wraps in the buffer at +104 of the size at +112
//!   [OFW-4.93 lv1 a:0x2F06D8-0x2F0710] [OFW-4.93 lv1 a:0x2F0780-0x2F07F0].
//! - After it copies one or more bytes, it stores the new read pointer and
//!   count [OFW-4.93 lv1 a:0x2F07FC-0x2F0810]. It then calls the listeners
//!   of the endpoint and of its peer [OFW-4.93 lv1 a:0x2F0830-0x2F0940],
//!   which compute the interrupt status that the section above gives.
//! - When the buffer holds no byte and the endpoint has a peer with a
//!   reference, it returns 0 with 0 bytes read, and changes nothing
//!   [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64] [OFW-4.93 lv1 a:0x2F0944-0x2F0948].
//! - When the buffer holds no byte and the endpoint has no such peer, the
//!   handler returns `LV1_WRONG_STATE` (-11), with r3 marked
//!   [OFW-4.93 lv1 a:0x2F0A68-0x2F0A70] [OFW-4.93 lv1 a:0x2F45E0-0x2F45F0]
//!   [OFW-4.93 lv1 a:0x2F464C-0x2F4658].
//!
//! So the answer to a call that passes the checks depends on the bytes in
//! the port's receive buffer. lv1's endpoint write function
//! [OFW-4.93 lv1 a:0x2F0494] puts bytes in that buffer. These callers
//! reach it:
//! - `write_virtual_uart` (hypercall 163), for the bytes that the
//!   partition sends to the port's peer [OFW-4.93 lv1 a:0x2F2D90];
//! - four functions of lv1's own code
//!   [OFW-4.93 lv1 a:0x2F31B4] [OFW-4.93 lv1 a:0x2F53F0]
//!   [OFW-4.93 lv1 a:0x2F59DC] [OFW-4.93 lv1 a:0x2F775C].
//!
//! One of these functions [OFW-4.93 lv1 a:0x2F30A0] serves a write whose
//! buffer lv1 translates through the function [OFW-4.93 lv1 a:0x29D074]
//! [OFW-4.93 lv1 a:0x2A9DB8] [OFW-4.93 lv1 a:0x2A9E20]. For most ports, no
//! image read shows when lv1's services make these writes, or which bytes
//! they write. The two service ports are the exception; see the section
//! on the service ports.
//!
//! # Writing a port
//!
//! The handler of hypercall 163 [OFW-4.93 lv1 a:0x2F2AF4] keeps r3, r4
//! and r5 [OFW-4.93 lv1 a:0x2F2B08] [OFW-4.93 lv1 a:0x2F2B2C-0x2F2B30].
//! It does these steps in this order:
//! 1. It refuses r3 above [`VIRTUAL_UART_PORT_MAX`]
//!    [OFW-4.93 lv1 a:0x2F2B90-0x2F2B94].
//! 2. It refuses r4 above [`VIRTUAL_UART_WRITE_ADDRESS_MAX`].
//! 3. It reads the port's endpoint from the slot of r3 in the table at
//!    +152 of the virtual UART object. It refuses a slot with no endpoint
//!    [OFW-4.93 lv1 a:0x2F2BA8-0x2F2C68].
//! 4. It looks up the memory region of r4 with the function
//!    [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F2C6C-0x2F2C98]. It
//!    makes the checks of `read_virtual_uart` on the region: the class
//!    [OFW-4.93 lv1 a:0x2F2C9C-0x2F2CAC], the r5 bytes from r4 to the end
//!    [OFW-4.93 lv1 a:0x2F2CB0-0x2F2CC0], and the page array at +176
//!    [OFW-4.93 lv1 a:0x2F2CC4-0x2F2CCC].
//! 5. It refuses an endpoint with no peer that has a reference
//!    [OFW-4.93 lv1 a:0x2F2CF8-0x2F2D3C] [OFW-4.93 lv1 a:0x2F2F24-0x2F2F28].
//! 6. For each page of the buffer, it gives the endpoint, the page's real
//!    address and the bytes left in the page to the write function
//!    [OFW-4.93 lv1 a:0x2F2D40-0x2F2D90]. It stops after a write of 0
//!    bytes, or when no byte is left [OFW-4.93 lv1 a:0x2F2D9C-0x2F2DF0].
//! 7. It stores the sum of the bytes in the return slot of r4, and returns
//!    0 [OFW-4.93 lv1 a:0x2F2DF4-0x2F2E00] [OFW-4.93 lv1 a:0x2F2E68].
//!
//! The return slot is the slot that `read_virtual_uart` writes
//! [OFW-4.93 lv1 a:0x2F452C-0x2F4538]. A refusal skips the store, and the
//! vector's entry zeroes the return slots [OFW-4.93 lv1 a:0x0C08]. So r4
//! returns 0 for a call that fails, and r5 to r10 return 0. A call with
//! r5 = 0 gives no byte to the write function, and returns r3 = 0 and
//! r4 = 0. See [`WRITE_VIRTUAL_UART_NO_PORT_R3`] for the r3 of each check.
//!
//! The write function [OFW-4.93 lv1 a:0x2F0494] writes to the receive
//! buffer of the endpoint's peer:
//! - When the endpoint has a peer with a reference, it gives the peer to
//!   the function [OFW-4.93 lv1 a:0x2F00F4]
//!   [OFW-4.93 lv1 a:0x2F04E4-0x2F0530].
//! - That function copies the lesser of the bytes asked for and the free
//!   bytes of the peer's buffer: the size at +112 less the count at +120
//!   [OFW-4.93 lv1 a:0x2F0290-0x2F02BC]. It copies to the write pointer at
//!   +88, which wraps in the buffer at +104
//!   [OFW-4.93 lv1 a:0x2F02C0-0x2F0318].
//! - After it copies one or more bytes, it stores the new write pointer
//!   and count [OFW-4.93 lv1 a:0x2F0324-0x2F0334]. It then calls the
//!   listeners of the peer [OFW-4.93 lv1 a:0x2F0358-0x2F03AC]. The write
//!   function then calls the listeners of the endpoint
//!   [OFW-4.93 lv1 a:0x2F0548-0x2F059C], which compute the interrupt
//!   status that the section on the interrupt status gives.
//! - When the endpoint has no such peer, or that function fails
//!   [OFW-4.93 lv1 a:0x2F0534-0x2F0538], it returns 7 when the byte at
//!   +128 of the endpoint is not 0, and 0x9FE5F901 when that byte is 0
//!   [OFW-4.93 lv1 a:0x2F062C-0x2F0644].
//!
//! So a call that passes the checks writes all r5 bytes when the peer's
//! receive buffer has r5 free bytes. The service that holds the peer
//! reads them; see the next section.
//!
//! # The service ports
//!
//! lv1 carries a process archive [OFW-4.93 lv1 a:0x400000-0x4000E4]. Two of
//! its files hold the peer of a port of the GameOS partition:
//! - `sysmgr_ss.fself`, lv1's system manager, holds port 2. Its entry is
//!   the second [OFW-4.93 lv1 a:0x400014-0x40001C].
//! - `ss_server3.fself` holds port 10 for its dispatcher manager. Its
//!   entry is the eighth [OFW-4.93 lv1 a:0x40005C-0x400064].
//!
//! Each service writes its port to reply to the partition's request, or on
//! an event that CellGov does not model. So each port's receive buffer is
//! empty at the partition's first read. The next two sections give the
//! evidence for each service.
//!
//! ## The system manager
//!
//! The system manager's boot function for a partition does these steps in
//! this order:
//! 1. It calls the port function [OFW-4.93 lv1 a:0x44E990]. That function
//!    constructs port 2 of the partition's virtual UART with a buffer of
//!    2048 bytes, through process system call 0x10084
//!    [OFW-4.93 lv1 a:0x44CF04-0x44CF24] [OFW-4.93 lv1 a:0x464BC8-0x464BD8].
//!    It then opens the port's peer as a file, keeps the descriptor, and
//!    gives the descriptor a receive function
//!    [OFW-4.93 lv1 a:0x44CF58-0x44CF9C].
//! 2. It activates the partition's logical PU [OFW-4.93 lv1 a:0x44E9B8].
//! 3. It tells two other services that the boot is done
//!    [OFW-4.93 lv1 a:0x44E9E8] [OFW-4.93 lv1 a:0x44B26C-0x44B2AC]. That
//!    step does not use the descriptor.
//!
//! The system manager writes the port only through the descriptor. Four
//! functions of its image read the descriptor:
//! - The send function writes a 16-byte header and a payload
//!   [OFW-4.93 lv1 a:0x44C4C4] [OFW-4.93 lv1 a:0x44C544-0x44C558].
//! - The receive function reads a header and a payload. It then calls the
//!   request handler of the header's service id through a table
//!   [OFW-4.93 lv1 a:0x44CD54] [OFW-4.93 lv1 a:0x44CD80-0x44CE44]
//!   [OFW-4.93 lv1 a:0x46FD60-0x46FF4C].
//! - One request handler writes more bytes after its reply
//!   [OFW-4.93 lv1 a:0x44DF70-0x44DF7C] [OFW-4.93 lv1 a:0x44DFEC-0x44DFF8].
//! - The teardown function closes the descriptor and destructs the port
//!   [OFW-4.93 lv1 a:0x44CE60-0x44CEC8].
//!
//! Each call of the send function sends one of these messages:
//! - a reply, from a request handler that the receive function calls;
//! - a command, service id 3, when the system manager stops the partition
//!   [OFW-4.93 lv1 a:0x44CC10-0x44CC1C] [OFW-4.93 lv1 a:0x44F54C];
//! - an external event, service id 4, of a power or thermal event
//!   [OFW-4.93 lv1 a:0x44CC80-0x44CC8C] [OFW-4.93 lv1 a:0x452FE8-0x452FFC].
//!
//! Linux `drivers/ps3/ps3-sys-manager.c` gives the same direction for each
//! of these service ids, in `enum ps3_sys_manager_service_id`. It gives the
//! codes 3, 4, 7 and 8 of the events in `enum ps3_sys_manager_event`.
//!
//! So the system manager writes nothing to port 2 before the partition
//! sends a request, unless a shutdown or an external event comes first.
//! CellGov models neither. The system manager keeps the peer's descriptor
//! open until the teardown.
//!
//! ## The system manager's requests
//!
//! The image `sysmgr_ss.fself` is fake-signed, and its two segments are
//! plaintext in lv1's image. Its SCE header gives a header size of 0x480
//! [OFW-4.93 lv1 a:0x4218C0-0x4218D4]. Its ELF program headers place the
//! segments at these addresses [OFW-4.93 lv1 a:0x421990-0x4219FC]:
//! - the code, at vaddr 0x80000000, at a:0x431D40;
//! - the data, at vaddr 0xC0000000, at a:0x471D40, with 0xCE88 bytes in the
//!   file of 0xDD58 bytes in memory.
//!
//! The entry descriptor gives the TOC 0xC0012F40
//! [OFW-4.93 lv1 a:0x472FF0-0x472FFC].
//!
//! The receive function [OFW-4.93 lv1 a:0x44CD54] does these steps in
//! this order:
//! 1. It fills a header with the version 1, the size 16 and 0 in each
//!    other field [OFW-4.93 lv1 a:0x44CD60-0x44CD7C]
//!    [OFW-4.93 lv1 a:0x44BA78-0x44BAA8].
//! 2. It reads [`SYSTEM_MANAGER_HEADER_BYTES`] bytes over the header with
//!    one read, process system call 9
//!    [OFW-4.93 lv1 a:0x44CD80-0x44CD94] [OFW-4.93 lv1 a:0x468DB0-0x468DBC].
//!    A read that returns another count ends the function with no handler
//!    [OFW-4.93 lv1 a:0x44CDB0-0x44CDB4].
//! 3. It reads the payload bytes that
//!    [`system_manager_payload_read`] gives, in a loop of reads
//!    [OFW-4.93 lv1 a:0x44CDDC-0x44CE10].
//! 4. It calls the request handler of [`system_manager_handler_index`]
//!    through the table [OFW-4.93 lv1 a:0x44CE14-0x44CE44]. The handler
//!    gets the payload and the tag; see [`system_manager_header_tag`].
//!
//! The table at TOC slot r2 - 29520 holds vaddr 0x8003E020, which is
//! a:0x46FD60 [OFW-4.93 lv1 a:0x47D930-0x47D934]. So no handler runs, and
//! no reply comes, until the partition sends the header and the payload
//! bytes that the receive function reads.
//!
//! The 4.93 kernel's function [OFW-4.93 lv2_kernel a:0x80000000002646d8]
//! sends a request of service id 9 with 8 payload bytes in one write of
//! 24 bytes [OFW-4.93 lv2_kernel a:0x80000000002646e0-0x800000000026471c].
//! It stores 1 in payload byte 0 and its r4 in payload byte 1. It stores
//! no other payload byte. In CellGov's boot, this request is the kernel's
//! first write to port 2, and its r4 is 2; see
//! [`GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE`].
//!
//! Entry 9 of the table [OFW-4.93 lv1 a:0x46FDA8-0x46FDAC] holds the
//! descriptor of the handler [OFW-4.93 lv1 a:0x44DE84]
//! [OFW-4.93 lv1 a:0x477760-0x47776C]. The handler reads payload byte 1
//! [OFW-4.93 lv1 a:0x44DEA0-0x44DEC4]. For the value 2, it replies with
//! service id 0x0B and 16 payload bytes
//! [OFW-4.93 lv1 a:0x44DF0C-0x44DF24] [OFW-4.93 lv1 a:0x44BAF4-0x44BB14].
//! The reply holds these fields:
//! - byte 0 is 2;
//! - bytes 1 and 2 are bytes 2 and 3 at vaddr 0xC000D35B
//!   [OFW-4.93 lv1 a:0x45242C-0x452440] [OFW-4.93 lv1 a:0x47E1B8-0x47E1BC];
//! - bytes 4 to 7 are the word at vaddr 0xC000D2D8
//!   [OFW-4.93 lv1 a:0x44DEF4] [OFW-4.93 lv1 a:0x44DF08]
//!   [OFW-4.93 lv1 a:0x47DA48-0x47DA4C];
//! - bytes 8 to 15 are the doubleword at +96 of the current partition's
//!   state object, or 0 with no such object
//!   [OFW-4.93 lv1 a:0x44DEE0-0x44DEF0] [OFW-4.93 lv1 a:0x44F780-0x44F7A0].
//!
//! Both vaddrs are past the 0xCE88 file bytes of the data segment. So the
//! system manager sets them while it runs, and no image read gives their
//! values. The 4.93 kernel keeps the reply's fields in its system manager
//! object, and a console capture of that object gives the reply; see
//! [`GAME_OS_SYSTEM_MANAGER_REPLIES`].
//!
//! For payload byte 1 = 1, the handler replies with service id 0x0A and 8
//! payload bytes. It then writes the bytes that the system manager keeps
//! for the partition [OFW-4.93 lv1 a:0x44DF2C-0x44DFC8]. At the first
//! GameOS boot after lv1 starts, it keeps 0 bytes; see
//! [`GAME_OS_SYSTEM_MANAGER_REPLIES`].
//!
//! ## The dispatcher manager
//!
//! The secure loader code of `ss_server3.fself` asks the dispatcher
//! manager for the partition's service port, port 10
//! [OFW-4.93 lv1 a:0x5B7E44-0x5B7E80]. The dispatcher manager constructs
//! the port with a buffer of 4096 bytes, through process system call
//! 0x10084 [OFW-4.93 lv1 a:0x5BF1F0-0x5BF1FC] [OFW-4.93 lv1 a:0x5CA8FC-0x5CA90C].
//!
//! The dispatcher manager relays requests and replies. Its receive loop
//! calls one of two functions for a socket that has data
//! [OFW-4.93 lv1 a:0x5BE5D4] [OFW-4.93 lv1 a:0x5BE5E8]:
//! - The port function reads the partition's packet and sends it to a
//!   socket that it connects to the packet's service
//!   [OFW-4.93 lv1 a:0x5BD888] [OFW-4.93 lv1 a:0x5BDC88]. In other cases,
//!   it replies to the port with a return value alone
//!   [OFW-4.93 lv1 a:0x5BD980] [OFW-4.93 lv1 a:0x5BDAFC].
//! - The service function reads a service's reply from such a socket, and
//!   writes it to the port [OFW-4.93 lv1 a:0x5BD3F0]
//!   [OFW-4.93 lv1 a:0x5BD524] [OFW-4.93 lv1 a:0x5BD540].
//!
//! The service function's send function has no other caller. Two other
//! socket write functions have no direct caller: tables of function
//! descriptors hold them [OFW-4.93 lv1 a:0x5D356C]
//! [OFW-4.93 lv1 a:0x5D3674]. The image read does not show which objects
//! call them. Each write to the port that the image read shows is a reply
//! to a request. So the evidence supports this reading: the dispatcher
//! manager writes nothing to port 10 before the partition sends a request.
//!
//! ## The dispatcher manager's requests
//!
//! The port function [OFW-4.93 lv1 a:0x5BD888] reads a request in this
//! order:
//! 1. It reads a header of [`DISPATCHER_HEADER_BYTES`] bytes from the port
//!    [OFW-4.93 lv1 a:0x5BD8B4-0x5BD8E4]. The read function calls the
//!    loop [OFW-4.93 lv1 a:0x5BF300-0x5BF318] that reads until it holds
//!    each byte that it asks for [OFW-4.93 lv1 a:0x5BEFE0-0x5BF058].
//! 2. It looks up the service id that the header gives; see
//!    [`dispatcher_header_service`].
//! 3. It reads or discards the request bytes that the header counts, and
//!    only then replies; see [`dispatcher_header_request_bytes`].
//!
//! Step 3 has three paths:
//! - For a service id that the lookup does not find, it discards the
//!   bytes [OFW-4.93 lv1 a:0x5BD930-0x5BD93C]. It then replies with a
//!   return value alone [OFW-4.93 lv1 a:0x5BD980].
//! - When it cannot get a buffer for the larger of the two sizes in the
//!   header, it discards the bytes [OFW-4.93 lv1 a:0x5BD994-0x5BD9D0]. It
//!   then replies with a return value alone [OFW-4.93 lv1 a:0x5BDAFC].
//! - Otherwise it reads the bytes [OFW-4.93 lv1 a:0x5BDA18-0x5BDA28], and
//!   sends them to a socket that it connects to the service.
//!
//! The discard function reads pieces of at most 64 bytes until it holds
//! each byte [OFW-4.93 lv1 a:0x5BD6A4-0x5BD71C]. So the dispatcher manager
//! writes nothing to port 10 until the partition sends the header and each
//! request byte that it counts. The reply then comes from the service, or
//! has a return value that depends on the dispatcher manager's own state
//! [OFW-4.93 lv1 a:0x5BD948-0x5BD97C]. The section on the reply to the
//! kernel's first request gives the one reply that CellGov holds.
//!
//! ## The kernel's reads
//!
//! At the 4.93 kernel's first read of a service port, the port's receive
//! buffer is empty, and the port has a peer with a reference. The capture
//! [`VIRTUAL_UART_PORT_CAPTURE`] gives each service port a transmit buffer
//! size other than 0 at the kernel's open. That value is 0 when the port
//! has no such peer; see [`VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`]. So lv1's
//! read function returns 0 bytes, and the handler returns r3 = 0 and
//! r4 = 0.
//!
//! The kernel reads each port through one function, which sends
//! hypercall 162 with the port from its port record
//! [OFW-4.93 lv2_kernel a:0x8000000000016b6c-0x8000000000016b7c]. Its
//! system manager loop opens port 2
//! [OFW-4.93 lv2_kernel a:0x8000000000264388], and reads a 16-byte message
//! header [OFW-4.93 lv2_kernel a:0x8000000000264bb0-0x8000000000264bc0].
//! CellGov's boot of the kernel makes its first read on port 10 before
//! its first `write_virtual_uart`. When lv1 reads 0 bytes, the kernel adds
//! the value 2 to the port's interrupt mask with hypercall 164, and waits
//! for the port's interrupt
//! [OFW-4.93 lv2_kernel a:0x8000000000016b9c-0x8000000000016be0]
//! [OFW-4.93 lv2_kernel a:0x8000000000016aa8-0x8000000000016ac8].
//!
//! ## The kernel's first request
//!
//! The kernel writes each port through one function, which sends
//! hypercall 163 with the port from its port record
//! [OFW-4.93 lv2_kernel a:0x8000000000016fc8-0x8000000000016fd8]. In
//! CellGov's boot, the kernel's first two calls write port 10: a header,
//! then the request bytes that it counts. The request object of the
//! kernel holds the request id and the service id; see
//! [`GAME_OS_FIRST_DISPATCHER_REQUEST`].
//!
//! The kernel's dispatcher client sends a request in this form:
//! - The client's constructor sets the client's request count to 0 and
//!   its port to 10 [OFW-4.93 lv2_kernel a:0x8000000000212408]
//!   [OFW-4.93 lv2_kernel a:0x8000000000212428]
//!   [OFW-4.93 lv2_kernel a:0x8000000000212440].
//! - The send function gives the low word of the service id, the request
//!   size and the reply size to the header function
//!   [OFW-4.93 lv2_kernel a:0x80000000002131b0-0x8000000000213208]. For
//!   the class of the first request, both sizes are 0x38
//!   [OFW-4.93 lv2_kernel a:0x8000000000229c34].
//! - The header function puts the request count in the word at +0, then
//!   increments the count. It puts the service id, the request size and
//!   the reply size in the words at +4, +8 and +12
//!   [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc]
//!   [OFW-4.93 lv2_kernel a:0x800000000021221c-0x8000000000212230].
//! - It writes the 16 header bytes, then the request bytes
//!   [OFW-4.93 lv2_kernel a:0x8000000000212284-0x80000000002122ac].
//! - The request bytes start with the request id and the service id, as
//!   big-endian doublewords
//!   [OFW-4.93 lv2_kernel a:0x8000000000212e10-0x8000000000212eac].
//!
//! So the client's first request has a header with tag 0, service
//! 0x19000, 0x38 request bytes and 0x38 reply bytes.
//!
//! ## The reply to the kernel's first request
//!
//! The dispatcher manager's map gives service 0x19000 the socket address
//! 0x24 [OFW-4.93 lv1 a:0x5D7100-0x5D710C]. The builder of the map reads
//! that table [OFW-4.93 lv1 a:0x5BECDC-0x5BED50]. `ss_server2.fself` binds
//! a socket to that address and listens on it
//! [OFW-4.93 lv1 a:0x56C484-0x56C4A0] [OFW-4.93 lv1 a:0x58CF48-0x58CFBC].
//!
//! `ss_server2` answers a request on one buffer, in this order:
//! 1. The request class reads the request bytes from the buffer
//!    [OFW-4.93 lv1 a:0x58DF7C-0x58DFB4].
//! 2. The class runs its handler [OFW-4.93 lv1 a:0x58DFB8-0x58DFD4].
//! 3. The class writes its reply fields to the same buffer
//!    [OFW-4.93 lv1 a:0x58E044-0x58E07C].
//!
//! For request 0x19002, the handler stores its worker's return value as
//! the status [OFW-4.93 lv1 a:0x56CC5C-0x56CC6C]. The worker builds the
//! 16 data bytes with the isolated SPU module `aim_spu_module.self`
//! [OFW-4.93 lv1 a:0x593850] [OFW-4.93 lv1 a:0x56E5C8-0x56E5E4]. The
//! module reads data of the console that another request gives
//! [OFW-4.93 lv1 a:0x56E454-0x56E46C]. So no image read gives the data
//! bytes.
//!
//! The class writes two fields of the reply:
//! - the status word, at [`DISPATCHER_REPLY_STATUS_OFFSET`]
//!   [OFW-4.93 lv1 a:0x58D67C-0x58D6A8];
//! - the 16 data bytes, at [`DISPATCHER_REPLY_DATA_OFFSET`]
//!   [OFW-4.93 lv1 a:0x56D234-0x56D248].
//!
//! Each other byte of the reply is the request byte at the same offset.
//!
//! The dispatcher manager keeps the tag and the reply size of the
//! request's header [OFW-4.93 lv1 a:0x5BDC08-0x5BDC18]
//! [OFW-4.93 lv1 a:0x5BC98C-0x5BC9A8]. When the service replies, it
//! writes a header to the port, and then the reply bytes that the header
//! counts [OFW-4.93 lv1 a:0x5BD4FC-0x5BD540]; see
//! [`dispatcher_reply_header`].
//!
//! The kernel reads these fields of the reply:
//! - The request id at +0. The base unmarshal refuses a reply with
//!   another id [OFW-4.93 lv2_kernel a:0x8000000000212d18-0x8000000000212d40].
//! - The status word at +16, which it keeps at +24 of the request object
//!   [OFW-4.93 lv2_kernel a:0x8000000000212d44-0x8000000000212d6c].
//! - The 16 data bytes at +40
//!   [OFW-4.93 lv2_kernel a:0x8000000000229ab0-0x8000000000229b38].
//!
//! The kernel keeps the status and the data bytes in a cache object. The
//! capture [`DISPATCHER_REPLY_CAPTURE`] reads that object on the console;
//! see [`GAME_OS_DISPATCHER_REPLIES`].
//!
//! ## The kernel's request for random bytes
//!
//! In CellGov's boot, the kernel's second request to the dispatcher
//! manager is request 0x5003 of service 0x5000; see
//! [`GAME_OS_RANDOM_DISPATCHER_REQUESTS`]. The kernel uses its reply in
//! this way:
//! - The request object's constructor puts the request id at +8 and the
//!   service id at +16
//!   [OFW-4.93 lv2_kernel a:0x8000000000227fa4-0x8000000000227fdc].
//! - The class unmarshal copies the 24 reply bytes at +40 to the caller's
//!   buffer [OFW-4.93 lv2_kernel a:0x8000000000227fe0-0x80000000002280dc].
//! - The caller keeps the first 20 bytes as the seed of the kernel's
//!   random number generator
//!   [OFW-4.93 lv2_kernel a:0x8000000000228424-0x800000000022858c]
//!   [OFW-4.93 lv2_kernel a:0x8000000000228600-0x8000000000228718]
//!   [OFW-4.93 lv2_kernel a:0x8000000000228788-0x80000000002287d0].
//!
//! The dispatcher manager's map gives service 0x5000 the socket address
//! 0x23 [OFW-4.93 lv1 a:0x5D70D0-0x5D70DC]. A process of lv1 binds a
//! socket to that address [OFW-4.93 lv1 a:0x4EA9AC-0x4EA9C8]. Its class
//! for request 0x5003 [OFW-4.93 lv1 a:0x506C10-0x506C3C] answers in this
//! order:
//! 1. The class keeps the address of the 24 bytes at +40 of the request
//!    buffer [OFW-4.93 lv1 a:0x5076A4-0x5076E8] [OFW-4.93 lv1 a:0x5279C4].
//! 2. The handler stores its worker's return value as the status
//!    [OFW-4.93 lv1 a:0x508D60-0x508D84].
//! 3. The worker writes the 24 bytes at that address
//!    [OFW-4.93 lv1 a:0x50B490-0x50B550].
//!
//! The worker gets the bytes from one of two sources:
//! - A generator that it seeds from the time base, when process system
//!   call 0x100C1 gives 1 for the keys `ss`, `fw`, `dummy` and `rnd`
//!   [OFW-4.93 lv1 a:0x50BB98-0x50BC44] [OFW-4.93 lv1 a:0x529864-0x5298A0]
//!   [OFW-4.93 lv1 a:0x50B4BC-0x50B50C].
//! - Otherwise, the isolated SPU module `sb_iso_spu_module.self`
//!   [OFW-4.93 lv1 a:0x51D674-0x51D718] [OFW-4.93 lv1 a:0x51DAD0-0x51DAE8]
//!   [OFW-4.93 lv1 a:0x53F6F8-0x53F70C]. The function that takes the 24
//!   bytes from the module's result logs as `sbm_get_rnd`
//!   [OFW-4.93 lv1 a:0x51DA00-0x51DA84] [OFW-4.93 lv1 a:0x53F6A8-0x53F6F4].
//!
//! So no image read gives the 24 bytes.

use crate::hw::ppu_mmu::SMALL_PAGE_LOG2;

/// The handler of hypercall 166 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartIrqHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `configure_virtual_uart_irq` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 166 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_IRQ_HANDLERS: &[VirtualUartIrqHandler] = &[VirtualUartIrqHandler {
    firmware: "4.93",
    handler: 0x2F2810,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_irq_handler(firmware: &str) -> Option<&'static VirtualUartIrqHandler> {
    VIRTUAL_UART_IRQ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest r3 that the handler looks up. It refuses an r3 above
/// 2^62 - 1, unsigned [OFW-4.93 lv1 a:0x2F289C-0x2F28A8].
pub const VIRTUAL_UART_IRQ_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The bytes of the bitmap.
///
/// The handler refuses a region with fewer than 32 bytes from r3 to its
/// end [OFW-4.93 lv1 a:0x2F2914-0x2F2924]. lv1 copies 32 bytes of its
/// shadow to the bitmap [OFW-4.93 lv1 a:0x2F1604-0x2F1620].
pub const VIRTUAL_UART_IRQ_BITMAP_BYTES: u64 = 32;

/// Whether the bitmap at `address` and its next byte are in one 4 KB page.
///
/// The handler takes the region's page size from the exponent at +160
/// [OFW-4.93 lv1 a:0x2F290C]. It refuses a bitmap whose first byte and
/// the byte after its end are in two pages
/// [OFW-4.93 lv1 a:0x2F2928-0x2F2944]. It tests the offset of r3 from the
/// region's base [OFW-4.93 lv1 a:0x2F2914]. Each PPE page size is
/// a multiple of 4 KB [CBE-Handbook p:88 s:4.2.6.6]. So for a region whose
/// base is 4 KB-aligned, such as the real mode region at 0, a bitmap that
/// passes this test passes lv1's test for each page size.
pub const fn virtual_uart_irq_bitmap_in_small_page(address: u64) -> bool {
    let page = !((1u64 << SMALL_PAGE_LOG2) - 1);
    address & page == address.wrapping_add(VIRTUAL_UART_IRQ_BITMAP_BYTES) & page
}

/// The doubleword of the virtual UART's shadow, and the bit in it, that
/// hold the interrupt of port `port`.
///
/// lv1's status function reads the port at +168 of the endpoint. It
/// takes doubleword port / 64 and bit 63 - port % 64, with bit 0 the
/// least significant [OFW-4.93 lv1 a:0x2F2214-0x2F2248]. So port 0 is the
/// most significant bit of the first doubleword.
pub const fn virtual_uart_shadow_bit(port: u8) -> (u64, u64) {
    let port = port as u64;
    (port / 64, 1 << (63 - port % 64))
}

/// The r3 of `configure_virtual_uart_irq` for an r3 that no memory region
/// of the partition holds: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2F2AB8-0x2F2AC4].
///
/// As with `allocate_memory`, bit 32 marks r3. The handler returns this
/// r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_IRQ_ADDRESS_MAX`];
/// - no region holds r3 [OFW-4.93 lv1 a:0x2F28F8];
/// - the region has no page array.
///
/// Past the checks, these answers depend on lv1's own state:
/// - The live check of step 3 returns this r3
///   [OFW-4.93 lv1 a:0x2F17C8-0x2F17D8] [OFW-4.93 lv1 a:0x2F29BC-0x2F29D4].
/// - A failed allocation returns `LV1_RESOURCE_SHORTAGE` (-2),
///   zero-extended [OFW-4.93 lv1 a:0x2F1494-0x2F14A0]
///   [OFW-4.93 lv1 a:0x2F2AA0-0x2F2AB4].
pub const CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `configure_virtual_uart_irq` for a region that fails one of
/// these checks: `LV1_TYPE_MISMATCH` (-8), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2ACC-0x2F2AD8]:
/// - the region's class;
/// - the [`VIRTUAL_UART_IRQ_BITMAP_BYTES`] bytes from r3 to the region's
///   end.
pub const CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3: u64 = 0x0000_0001_FFFF_FFF8;

/// The r3 of `configure_virtual_uart_irq` for a bitmap whose first byte
/// and the byte after its end are in two pages:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked
/// [OFW-4.93 lv1 a:0x2F2AE0-0x2F2AEC].
pub const CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The r3 of `configure_virtual_uart_irq` for a partition whose virtual
/// UART already has an irq outlet: `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x2F2990-0x2F2994] [OFW-4.93 lv1 a:0x2F2A90-0x2F2A98].
pub const CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3: u64 = 0x0000_0000_FFFF_FFF5;

/// The console capture of the outlet id of the 4.93 kernel's call.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_IRQ_OUTLET_CAPTURE: &str = "tests/micro/lv2_vuart_irq_outlet";

/// The `configure_virtual_uart_irq` call of the GameOS kernel's boot at
/// one firmware, and the outlet id that lv1 gave it on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartIrqCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The outlet id in r4 of lv1's answer.
    pub outlet: u64,
}

/// The kernel's call for each firmware that a source gives the outlet id
/// for.
///
/// The partition's other objects decide the id, so no image read gives
/// it. The 4.93 kernel makes the call in the function
/// [OFW-4.93 lv2_kernel a:0x8000000000017ad0], and calls that function
/// again until it returns 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017cac-0x8000000000017cb8]. The
/// capture [`VIRTUAL_UART_IRQ_OUTLET_CAPTURE`] reads the id where the
/// kernel stores it after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017b10-0x8000000000017b20].
pub const GAME_OS_VIRTUAL_UART_IRQ_CALLS: &[GameOsVirtualUartIrqCall] =
    &[GameOsVirtualUartIrqCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7b0c,
        outlet: 18,
    }];

/// The outlet id that lv1 gave the GameOS kernel's call at the `sc`
/// address `sc`.
///
/// The value is `None` when no source gives that id at the firmware
/// version `firmware`.
pub fn game_os_virtual_uart_outlet(firmware: &str, sc: u64) -> Option<u64> {
    GAME_OS_VIRTUAL_UART_IRQ_CALLS
        .iter()
        .find(|call| call.firmware == firmware && call.sc == sc)
        .map(|call| call.outlet)
}

/// The handler of hypercall 165 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `get_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 165 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_PARAM_HANDLERS: &[VirtualUartParamHandler] = &[VirtualUartParamHandler {
    firmware: "4.93",
    handler: 0x2F3A68,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_param_handler(firmware: &str) -> Option<&'static VirtualUartParamHandler> {
    VIRTUAL_UART_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest port number. The worker refuses an r3 above 255, unsigned
/// [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
pub const VIRTUAL_UART_PORT_MAX: u64 = 0xFF;

/// The largest parameter id. The worker refuses an r4 above 7, unsigned
/// [OFW-4.93 lv1 a:0x2F35C4-0x2F35C8].
pub const VIRTUAL_UART_PARAM_MAX: u64 = 7;

/// The parameter id of the port's receive buffer size,
/// `PARAM_RX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 3 reads +112 of the port's endpoint
/// [OFW-4.93 lv1 a:0x2F3758-0x2F3764].
pub const VIRTUAL_UART_PARAM_RX_BUFFER_SIZE: u64 = 3;

/// The parameter id of the port's transmit buffer size,
/// `PARAM_TX_BUF_SIZE` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 5 reads +112 of the endpoint at +72 of the
/// port's endpoint: the receive buffer size of the peer that the port
/// sends to. It gives the value 0, with r3 = 0, when the port has no peer,
/// or when bits 2 to 31 of the word at +8 of the peer are 0
/// [OFW-4.93 lv1 a:0x2F37BC-0x2F3828] [OFW-4.93 lv1 a:0x2F399C-0x2F39A0].
/// So the value changes when lv1 links the port to another peer.
pub const VIRTUAL_UART_PARAM_TX_BUFFER_SIZE: u64 = 5;

/// The parameter id of the port's interrupt status,
/// `PARAM_INTERRUPT_STATUS` in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker's entry for 7 locks the endpoint at +80. It then gives the
/// endpoint to the function [OFW-4.93 lv1 a:0x2F0CC4], and stores that
/// function's r3 as the value [OFW-4.93 lv1 a:0x2F3900-0x2F3954]
/// [OFW-4.93 lv1 a:0x2F367C-0x2F36A0]. That function computes the bits of
/// [`virtual_uart_status`] from the endpoint and its peer
/// [OFW-4.93 lv1 a:0x2F0D00-0x2F0D7C]. It gives 0 when the byte at +128 of
/// the endpoint is not 0 [OFW-4.93 lv1 a:0x2F0CC8-0x2F0CD4]. So the value
/// changes with the port's traffic and its triggers.
pub const VIRTUAL_UART_PARAM_INTERRUPT_STATUS: u64 = 7;

/// The r3 of `get_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F3958-0x2F3968].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const GET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `get_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in the
/// high word [OFW-4.93 lv1 a:0x2F35CC-0x2F35D8].
pub const GET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The console capture of the buffer sizes of the 4.93 kernel's ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_CAPTURE: &str = "tests/micro/lv2_vuart_port_params";

/// A port of the GameOS partition's virtual UART at one firmware, and
/// the buffer sizes that lv1 gave the kernel's calls on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartPort {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The port number in r3 of the call.
    pub port: u64,
    /// The value of [`VIRTUAL_UART_PARAM_RX_BUFFER_SIZE`].
    pub rx_buffer_size: u64,
    /// The value of [`VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`].
    pub tx_buffer_size: u64,
}

/// The ports that the GameOS kernel opens, for each firmware that a source
/// gives them for, by ascending port number.
///
/// The 4.93 kernel's open function reads both sizes with hypercall 165
/// [OFW-4.93 lv2_kernel a:0x8000000000017708-0x8000000000017750]. It stores
/// them in its port record only after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017740]
/// [OFW-4.93 lv2_kernel a:0x800000000001775c]. The capture
/// [`VIRTUAL_UART_PORT_CAPTURE`] reads them from the records of the
/// three ports that the kernel opened.
pub const GAME_OS_VIRTUAL_UART_PORTS: &[GameOsVirtualUartPort] = &[
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 0,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 2,
        rx_buffer_size: 0x800,
        tx_buffer_size: 0x800,
    },
    GameOsVirtualUartPort {
        firmware: "4.93",
        port: 10,
        rx_buffer_size: 0x1000,
        tx_buffer_size: 0x1000,
    },
];

/// The GameOS partition's port `port` at the firmware version `firmware`.
///
/// The value is `None` when no source gives that port at `firmware`.
pub fn game_os_virtual_uart_port(
    firmware: &str,
    port: u64,
) -> Option<&'static GameOsVirtualUartPort> {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == firmware && row.port == port)
}

/// A `get_virtual_uart_param` call of the GameOS kernel's open function at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsVirtualUartParamCall {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The parameter id in r4 of the call.
    pub param: u64,
}

/// The kernel's calls for the buffer sizes, for each firmware that a
/// source gives them for, by ascending `sc` address.
///
/// The capture [`VIRTUAL_UART_PORT_CAPTURE`] reads lv1's answers to these
/// calls, and to no other call. The 4.93 kernel's open function sends 165
/// with r4 = 3 [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017730],
/// then with r4 = 5
/// [OFW-4.93 lv2_kernel a:0x8000000000017744-0x8000000000017750].
pub const GAME_OS_VIRTUAL_UART_PARAM_CALLS: &[GameOsVirtualUartParamCall] = &[
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7730,
        param: VIRTUAL_UART_PARAM_RX_BUFFER_SIZE,
    },
    GameOsVirtualUartParamCall {
        firmware: "4.93",
        sc: 0x8000_0000_0001_7750,
        param: VIRTUAL_UART_PARAM_TX_BUFFER_SIZE,
    },
];

/// Whether the GameOS kernel's open function asks for parameter `param` at
/// the `sc` address `sc` at the firmware version `firmware`.
pub fn is_game_os_virtual_uart_param_call(firmware: &str, sc: u64, param: u64) -> bool {
    GAME_OS_VIRTUAL_UART_PARAM_CALLS
        .iter()
        .any(|call| call.firmware == firmware && call.sc == sc && call.param == param)
}

/// The handler of hypercall 164 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartSetParamHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `set_virtual_uart_param` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 164 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_SET_PARAM_HANDLERS: &[VirtualUartSetParamHandler] =
    &[VirtualUartSetParamHandler {
        firmware: "4.93",
        handler: 0x2F4100,
    }];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_set_param_handler(
    firmware: &str,
) -> Option<&'static VirtualUartSetParamHandler> {
    VIRTUAL_UART_SET_PARAM_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The parameter id of the port's transmit trigger, `PARAM_TX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +136 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3D0C-0x2F3D4C].
pub const VIRTUAL_UART_PARAM_TX_TRIGGER: u64 = 0;

/// The parameter id of the port's receive trigger, `PARAM_RX_TRIGGER` in
/// Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +144 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D00] [OFW-4.93 lv1 a:0x2F3E58-0x2F3E94].
pub const VIRTUAL_UART_PARAM_RX_TRIGGER: u64 = 1;

/// The parameter id of the port's interrupt mask, `PARAM_INTERRUPT_MASK`
/// in Linux `drivers/ps3/ps3-vuart.c`.
///
/// The worker stores r5 for this id at +152 of the endpoint
/// [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF4] [OFW-4.93 lv1 a:0x2F3F4C-0x2F3F8C].
pub const VIRTUAL_UART_PARAM_INTERRUPT_MASK: u64 = 2;

/// The largest parameter id that `set_virtual_uart_param` sets. The
/// worker refuses an r4 above 2, unsigned
/// [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3DF8].
pub const VIRTUAL_UART_SET_PARAM_MAX: u64 = VIRTUAL_UART_PARAM_INTERRUPT_MASK;

/// The largest interrupt mask. The worker refuses an r5 with a bit above
/// bit 2 for the mask [OFW-4.93 lv1 a:0x2F3F40-0x2F3F48]. The status
/// bits that a mask can match are 1, 2 and 4; see [`crate::lv1::uart`].
pub const VIRTUAL_UART_INTERRUPT_MASK_MAX: u64 = 7;

/// The status bit of a port whose peer's receive buffer holds no more
/// bytes than the transmit trigger: `INTERRUPT_MASK_TX` in Linux
/// `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_TX: u64 = 1;

/// The status bit of a port whose receive buffer has no more free bytes
/// than the receive trigger: `INTERRUPT_MASK_RX` in Linux
/// `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_RX: u64 = 2;

/// The status bit of a port with no peer that has a reference:
/// `INTERRUPT_MASK_DISCONNECT` in Linux `drivers/ps3/ps3-vuart.c`.
pub const VIRTUAL_UART_STATUS_DISCONNECT: u64 = 4;

/// The status bit of a port with a peer that has a reference. No mask
/// that lv1 accepts holds this bit.
pub const VIRTUAL_UART_STATUS_CONNECTED: u64 = 8;

/// The fields of a port's endpoint, other than the triggers, that lv1's
/// status function reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartTraffic {
    /// The size of the receive buffer, at +112 of the endpoint.
    pub buffer_size: u64,
    /// The bytes that the receive buffer holds, at +120 of the endpoint.
    pub buffer_count: u64,
    /// The bytes that the peer's receive buffer holds, at +120 of the
    /// peer. The value is `None` when the port has no peer with a
    /// reference.
    pub peer_count: Option<u64>,
}

/// The interrupt status that lv1's status function computes for a port
/// with `traffic` and the triggers `tx_trigger` and `rx_trigger`.
///
/// The function [OFW-4.93 lv1 a:0x2F20E4] sets these bits:
/// - [`VIRTUAL_UART_STATUS_DISCONNECT`] when the peer at +72 is 0, or the
///   peer's reference count is 0 [OFW-4.93 lv1 a:0x2F21B0-0x2F21E8];
/// - [`VIRTUAL_UART_STATUS_CONNECTED`] for a peer with a reference, and
///   [`VIRTUAL_UART_STATUS_TX`] too when the peer's count is not above
///   the transmit trigger, unsigned [OFW-4.93 lv1 a:0x2F21F4-0x2F2204]
///   [OFW-4.93 lv1 a:0x2F2380-0x2F2384];
/// - [`VIRTUAL_UART_STATUS_RX`] when the buffer size less the count is not
///   above the receive trigger, unsigned
///   [OFW-4.93 lv1 a:0x2F2208-0x2F2210] [OFW-4.93 lv1 a:0x2F2388-0x2F238C]
///   [OFW-4.93 lv1 a:0x2F2348].
///
/// The function computes the status after each call of
/// `set_virtual_uart_param` that succeeds, the mask call included
/// [OFW-4.93 lv1 a:0x2F3FB0-0x2F4008]. It sets the port's shadow bit only
/// when the status and the mask share a bit
/// [OFW-4.93 lv1 a:0x2F221C-0x2F224C]; see [`crate::lv1::uart`].
pub const fn virtual_uart_status(
    traffic: VirtualUartTraffic,
    tx_trigger: u64,
    rx_trigger: u64,
) -> u64 {
    let mut status = match traffic.peer_count {
        None => VIRTUAL_UART_STATUS_DISCONNECT,
        Some(peer_count) if peer_count <= tx_trigger => {
            VIRTUAL_UART_STATUS_CONNECTED | VIRTUAL_UART_STATUS_TX
        }
        Some(_) => VIRTUAL_UART_STATUS_CONNECTED,
    };
    if traffic.buffer_size.wrapping_sub(traffic.buffer_count) <= rx_trigger {
        status |= VIRTUAL_UART_STATUS_RX;
    }
    status
}

/// The r3 of `set_virtual_uart_param` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4038-0x2F4044].
///
/// The worker returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3.
pub const SET_VIRTUAL_UART_PARAM_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a parameter id above
/// [`VIRTUAL_UART_SET_PARAM_MAX`]: `LV1_NO_ENTRY` (-6), with r4 marked in
/// the high word [OFW-4.93 lv1 a:0x2F3DF8-0x2F3E04].
pub const SET_VIRTUAL_UART_PARAM_NO_PARAM_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `set_virtual_uart_param` for a mask above
/// [`VIRTUAL_UART_INTERRUPT_MASK_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE`
/// (-17), with r5 marked in the high word
/// [OFW-4.93 lv1 a:0x2F40EC-0x2F40F8].
pub const SET_VIRTUAL_UART_PARAM_MASK_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The console capture of the interrupt mask that the 4.93 kernel keeps
/// for each of its ports.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const VIRTUAL_UART_PORT_MASK_CAPTURE: &str = "tests/micro/lv2_vuart_port_mask";

/// The handler of hypercall 162 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartReadHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `read_virtual_uart` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 162 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_READ_HANDLERS: &[VirtualUartReadHandler] = &[VirtualUartReadHandler {
    firmware: "4.93",
    handler: 0x2F429C,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_read_handler(firmware: &str) -> Option<&'static VirtualUartReadHandler> {
    VIRTUAL_UART_READ_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest buffer address that `read_virtual_uart` looks up. The
/// handler refuses an r4 above 2^62 - 1, unsigned
/// [OFW-4.93 lv1 a:0x2F4344-0x2F4350].
pub const VIRTUAL_UART_READ_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The r3 of `read_virtual_uart` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4604-0x2F4610].
///
/// The handler returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3;
/// - the read function returns 7, because the byte at +128 of the
///   endpoint is not 0 [OFW-4.93 lv1 a:0x2F45D8-0x2F45DC].
pub const READ_VIRTUAL_UART_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `read_virtual_uart` for a buffer address that no memory
/// region holds: `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F4618-0x2F4624].
///
/// The handler returns this r3 in these cases:
/// - r4 is above [`VIRTUAL_UART_READ_ADDRESS_MAX`];
/// - no region holds r4;
/// - the region has no page array.
pub const READ_VIRTUAL_UART_NO_REGION_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `read_virtual_uart` for a region that fails one of these
/// checks: `LV1_TYPE_MISMATCH` (-8), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F462C-0x2F4638]:
/// - the region's class;
/// - the r5 bytes from r4 to the region's end.
pub const READ_VIRTUAL_UART_REGION_R3: u64 = 0x0000_0002_FFFF_FFF8;

/// The handler of hypercall 163 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualUartWriteHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `write_virtual_uart` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 163 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const VIRTUAL_UART_WRITE_HANDLERS: &[VirtualUartWriteHandler] = &[VirtualUartWriteHandler {
    firmware: "4.93",
    handler: 0x2F2AF4,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn virtual_uart_write_handler(firmware: &str) -> Option<&'static VirtualUartWriteHandler> {
    VIRTUAL_UART_WRITE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The largest buffer address that `write_virtual_uart` looks up. The
/// handler refuses an r4 above 2^62 - 1, unsigned
/// [OFW-4.93 lv1 a:0x2F2B98-0x2F2BA4].
pub const VIRTUAL_UART_WRITE_ADDRESS_MAX: u64 = 0x3FFF_FFFF_FFFF_FFFF;

/// The r3 of `write_virtual_uart` for a port that lv1 does not hold:
/// `LV1_NO_ENTRY` (-6), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EC8-0x2F2ED4].
///
/// The handler returns this r3 in these cases:
/// - r3 is above [`VIRTUAL_UART_PORT_MAX`];
/// - the table holds no endpoint for r3;
/// - the write function returns 7 before it writes a byte
///   [OFW-4.93 lv1 a:0x2F2E9C-0x2F2EA0].
///
/// Another failure of the write function before it writes a byte returns
/// `LV1_WRONG_STATE` for 0x9FE5F901, or 0xFFFF8000 for other values
/// [OFW-4.93 lv1 a:0x2F2EA4-0x2F2EC4]. A failure after it writes one or
/// more bytes returns 0 with the sum in r4
/// [OFW-4.93 lv1 a:0x2F2E94-0x2F2E98].
pub const WRITE_VIRTUAL_UART_NO_PORT_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 of `write_virtual_uart` for a buffer address that no memory
/// region holds: `LV1_NO_ENTRY` (-6), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EF0-0x2F2EFC].
///
/// The handler returns this r3 in these cases:
/// - r4 is above [`VIRTUAL_UART_WRITE_ADDRESS_MAX`];
/// - no region holds r4 [OFW-4.93 lv1 a:0x2F2C94-0x2F2C98];
/// - the region has no page array [OFW-4.93 lv1 a:0x2F2CC8-0x2F2CCC].
pub const WRITE_VIRTUAL_UART_NO_REGION_R3: u64 = 0x0000_0002_FFFF_FFFA;

/// The r3 of `write_virtual_uart` for a region that fails one of these
/// checks: `LV1_TYPE_MISMATCH` (-8), with r4 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2F10-0x2F2F1C]:
/// - the region's class;
/// - the r5 bytes from r4 to the region's end.
pub const WRITE_VIRTUAL_UART_REGION_R3: u64 = 0x0000_0002_FFFF_FFF8;

/// The r3 of `write_virtual_uart` for a port with no peer that has a
/// reference: `LV1_WRONG_STATE` (-11), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2F2EDC-0x2F2EE8].
pub const WRITE_VIRTUAL_UART_NO_PEER_R3: u64 = 0x0000_0001_FFFF_FFF5;

/// An lv1 service that holds the peer of a port of the GameOS partition's
/// virtual UART.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualUartService {
    /// The system manager, `sysmgr_ss.fself` of lv1's process archive.
    SystemManager,
    /// The dispatcher manager of `ss_server3.fself` of lv1's process
    /// archive.
    DispatcherManager,
}

/// A port of the GameOS partition's virtual UART that an lv1 service
/// constructs at one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsServicePort {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The port number.
    pub port: u64,
    /// The service that holds the port's peer.
    pub service: VirtualUartService,
    /// The bytes of the buffer that the service gives the port.
    pub buffer_bytes: u64,
}

/// The service ports for each firmware whose lv1 image shows them, by
/// ascending port number.
///
/// Each service gives process system call 0x10084 the port and the buffer
/// size:
/// - the 4.93 system manager, port 2 and 2048 bytes
///   [OFW-4.93 lv1 a:0x44CF04-0x44CF0C] [OFW-4.93 lv1 a:0x44CF24];
/// - the 4.93 dispatcher manager, the port that the secure loader code
///   gives it and 4096 bytes [OFW-4.93 lv1 a:0x5BF1F8-0x5BF1FC]. That port
///   is 10 [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
///
/// Each port's receive buffer holds no byte at the GameOS kernel's first
/// read; see [`crate::lv1::uart`].
pub const GAME_OS_SERVICE_PORTS: &[GameOsServicePort] = &[
    GameOsServicePort {
        firmware: "4.93",
        port: 2,
        service: VirtualUartService::SystemManager,
        buffer_bytes: 0x800,
    },
    GameOsServicePort {
        firmware: "4.93",
        port: 10,
        service: VirtualUartService::DispatcherManager,
        buffer_bytes: 0x1000,
    },
];

/// The service port `port` at the firmware version `firmware`, or `None`
/// when no lv1 image read shows a service that holds its peer.
pub fn game_os_service_port(firmware: &str, port: u64) -> Option<&'static GameOsServicePort> {
    GAME_OS_SERVICE_PORTS
        .iter()
        .find(|row| row.firmware == firmware && row.port == port)
}

/// The bytes of the header of a request that the dispatcher manager reads
/// from its port first [OFW-4.93 lv1 a:0x5BD8B8-0x5BD8BC].
pub const DISPATCHER_HEADER_BYTES: u64 = 16;

/// The service id that the request header `header` gives: its big-endian
/// word at +4.
///
/// The port function looks up that word [OFW-4.93 lv1 a:0x5BD910-0x5BD920].
pub const fn dispatcher_header_service(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[4], header[5], header[6], header[7]])
}

/// The count of request bytes after the request header `header`: its
/// big-endian word at +8.
///
/// The port function gives that word to the discard function
/// [OFW-4.93 lv1 a:0x5BD934-0x5BD93C], and to the read of the request
/// bytes [OFW-4.93 lv1 a:0x5BDA18-0x5BDA28].
pub const fn dispatcher_header_request_bytes(
    header: &[u8; DISPATCHER_HEADER_BYTES as usize],
) -> u32 {
    u32::from_be_bytes([header[8], header[9], header[10], header[11]])
}

/// The service id of the 4.93 GameOS kernel's first request to the
/// dispatcher manager.
///
/// The constructor of the kernel's request object stores it at +16
/// [OFW-4.93 lv2_kernel a:0x8000000000229950]
/// [OFW-4.93 lv2_kernel a:0x800000000022995c].
pub const GAME_OS_FIRST_DISPATCHER_SERVICE: u32 = 0x19000;

/// The request id of the 4.93 GameOS kernel's first request to the
/// dispatcher manager.
///
/// The constructor of the kernel's request object stores it at +8
/// [OFW-4.93 lv2_kernel a:0x800000000022994c]
/// [OFW-4.93 lv2_kernel a:0x8000000000229958]. In CellGov's boot, the
/// request bytes that the kernel sends start with this id, as a
/// big-endian doubleword.
pub const GAME_OS_FIRST_DISPATCHER_REQUEST: u64 = 0x19002;

/// The tag of the request header `header`: its big-endian word at +0.
///
/// The port function keeps the tag for the reply
/// [OFW-4.93 lv1 a:0x5BDC0C] [OFW-4.93 lv1 a:0x5BC9A4].
pub const fn dispatcher_header_tag(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[0], header[1], header[2], header[3]])
}

/// The count of reply bytes that the request header `header` gives: its
/// big-endian word at +12.
///
/// The port function keeps the count for the reply
/// [OFW-4.93 lv1 a:0x5BDC08] [OFW-4.93 lv1 a:0x5BC990].
pub const fn dispatcher_header_reply_bytes(header: &[u8; DISPATCHER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[12], header[13], header[14], header[15]])
}

/// The header that the dispatcher manager writes to its port before a
/// service's reply to a request with the tag `tag` and `reply_bytes`
/// reply bytes.
///
/// The service function puts the tag in the word at +0, 0 in the words at
/// +4 and +8, and the count in the word at +12
/// [OFW-4.93 lv1 a:0x5BD4FC-0x5BD520]. It takes that path only when its
/// receive call returns 0 in r31 [OFW-4.93 lv1 a:0x5BD464-0x5BD474].
pub const fn dispatcher_reply_header(
    tag: u32,
    reply_bytes: u32,
) -> [u8; DISPATCHER_HEADER_BYTES as usize] {
    let t = tag.to_be_bytes();
    let n = reply_bytes.to_be_bytes();
    [
        t[0], t[1], t[2], t[3], 0, 0, 0, 0, 0, 0, 0, 0, n[0], n[1], n[2], n[3],
    ]
}

/// The offset of the 4-byte status word in the reply bytes of a request
/// that `ss_server2.fself` answers [OFW-4.93 lv1 a:0x58D684-0x58D6A8].
pub const DISPATCHER_REPLY_STATUS_OFFSET: usize = 16;

/// The offset of the 16 data bytes in the reply bytes of request 0x19002
/// [OFW-4.93 lv1 a:0x58D59C] [OFW-4.93 lv1 a:0x56D234-0x56D248].
pub const DISPATCHER_REPLY_DATA_OFFSET: usize = 40;

/// The bytes of the data field of the reply to request 0x19002
/// [OFW-4.93 lv1 a:0x56D244].
pub const DISPATCHER_REPLY_DATA_BYTES: usize = 16;

/// The console capture of the reply that the 4.93 kernel kept from its
/// first dispatcher manager request.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const DISPATCHER_REPLY_CAPTURE: &str = "tests/micro/lv2_aim_device_type";

/// The reply of a service of lv1 to one request of the GameOS kernel, at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDispatcherReply {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The service id that the request header gives.
    pub service: u32,
    /// The request id that the request bytes start with.
    pub request: u64,
    /// The count of request bytes and the count of reply bytes that the
    /// request header gives.
    pub bytes: u32,
    /// The status word at [`DISPATCHER_REPLY_STATUS_OFFSET`].
    pub status: u32,
    /// The data bytes at [`DISPATCHER_REPLY_DATA_OFFSET`].
    pub data: [u8; DISPATCHER_REPLY_DATA_BYTES],
}

/// Each reply that a source gives.
///
/// The 4.93 kernel's first request has 0x38 request bytes and 0x38 reply
/// bytes; see [`GAME_OS_FIRST_DISPATCHER_REQUEST`]. The capture
/// [`DISPATCHER_REPLY_CAPTURE`] gives its status and data bytes. The data
/// bytes are the value of the reference console's class. No image read
/// gives them; see [`crate::lv1::uart`].
pub const GAME_OS_DISPATCHER_REPLIES: &[GameOsDispatcherReply] = &[GameOsDispatcherReply {
    firmware: "4.93",
    service: GAME_OS_FIRST_DISPATCHER_SERVICE,
    request: GAME_OS_FIRST_DISPATCHER_REQUEST,
    bytes: 0x38,
    status: 0,
    data: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x84],
}];

/// The reply to request `request` of service `service` at the firmware
/// version `firmware`, or `None` when no source gives it.
pub fn game_os_dispatcher_reply(
    firmware: &str,
    service: u32,
    request: u64,
) -> Option<&'static GameOsDispatcherReply> {
    GAME_OS_DISPATCHER_REPLIES
        .iter()
        .find(|row| row.firmware == firmware && row.service == service && row.request == request)
}

/// The bytes of random data in the reply to a request of
/// [`GAME_OS_RANDOM_DISPATCHER_REQUESTS`], at
/// [`DISPATCHER_REPLY_DATA_OFFSET`]
/// [OFW-4.93 lv2_kernel a:0x8000000000227fe0-0x80000000002280dc]
/// [OFW-4.93 lv1 a:0x50B54C].
pub const DISPATCHER_RANDOM_DATA_BYTES: usize = 24;

/// A request of the GameOS kernel whose reply holds random bytes that lv1
/// draws, at one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsRandomDispatcherRequest {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The service id that the request header gives.
    pub service: u32,
    /// The request id that the request bytes start with.
    pub request: u64,
    /// The count of request bytes and the count of reply bytes that the
    /// request header gives.
    pub bytes: u32,
}

/// Each request whose reply data an lv1 image read shows to be random;
/// see [`crate::lv1::uart`].
///
/// The 4.93 kernel's class gives 0x40 as both counts
/// [OFW-4.93 lv2_kernel a:0x8000000000228178]. It sets the request id
/// 0x5003 and the service id 0x5000
/// [OFW-4.93 lv2_kernel a:0x8000000000227fb0-0x8000000000227fc0].
pub const GAME_OS_RANDOM_DISPATCHER_REQUESTS: &[GameOsRandomDispatcherRequest] =
    &[GameOsRandomDispatcherRequest {
        firmware: "4.93",
        service: 0x5000,
        request: 0x5003,
        bytes: 0x40,
    }];

/// The request `request` of service `service` at the firmware version
/// `firmware`, or `None` when no image read shows its reply data to be
/// random.
pub fn game_os_random_dispatcher_request(
    firmware: &str,
    service: u32,
    request: u64,
) -> Option<&'static GameOsRandomDispatcherRequest> {
    GAME_OS_RANDOM_DISPATCHER_REQUESTS
        .iter()
        .find(|row| row.firmware == firmware && row.service == service && row.request == request)
}

/// The bytes of the header of a request that the system manager reads
/// from its port first, with one read
/// [OFW-4.93 lv1 a:0x44CD84-0x44CD94].
pub const SYSTEM_MANAGER_HEADER_BYTES: u64 = 16;

/// The count of payload bytes that the system manager's request header
/// `header` gives: its big-endian word at +4.
///
/// The receive function loads that word for the payload read
/// [OFW-4.93 lv1 a:0x44CDB8]. Linux `struct ps3_sys_manager_header` names
/// it `payload_size`.
pub const fn system_manager_header_payload_bytes(
    header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize],
) -> u32 {
    u32::from_be_bytes([header[4], header[5], header[6], header[7]])
}

/// The service id that the system manager's request header `header`
/// gives: its big-endian halfword at +8.
///
/// The receive function loads that halfword for its log call
/// [OFW-4.93 lv1 a:0x44CD9C]. Linux `struct ps3_sys_manager_header` names
/// it `service_id`.
pub const fn system_manager_header_service(
    header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize],
) -> u16 {
    u16::from_be_bytes([header[8], header[9]])
}

/// The tag of the system manager's request header `header`: its
/// big-endian word at +12.
///
/// The receive function gives that word to the request handler in r4
/// [OFW-4.93 lv1 a:0x44CE30]. Linux `struct ps3_sys_manager_header` names
/// it `request_tag`.
pub const fn system_manager_header_tag(header: &[u8; SYSTEM_MANAGER_HEADER_BYTES as usize]) -> u32 {
    u32::from_be_bytes([header[12], header[13], header[14], header[15]])
}

/// The largest count of payload bytes that the system manager's receive
/// function reads after one header [OFW-4.93 lv1 a:0x44CDBC-0x44CDC4].
pub const SYSTEM_MANAGER_PAYLOAD_MAX: u32 = 2048;

/// The count of payload bytes that the system manager's receive function
/// reads after a header that counts `payload_bytes`.
///
/// The function compares the count as a signed word
/// [OFW-4.93 lv1 a:0x44CDB8-0x44CDD8]. The value is one of these:
/// - `None` for 0: the function ends with no handler and no reply;
/// - 0 for a negative count: the read loop reads no byte, and the handler
///   runs [OFW-4.93 lv1 a:0x44CDFC-0x44CE10];
/// - [`SYSTEM_MANAGER_PAYLOAD_MAX`] for a count above it. The port keeps
///   the other bytes, and the next call reads them as a header;
/// - the count for each other value.
pub const fn system_manager_payload_read(payload_bytes: u32) -> Option<u32> {
    let count = payload_bytes as i32;
    if count == 0 {
        None
    } else if count < 0 {
        Some(0)
    } else if payload_bytes > SYSTEM_MANAGER_PAYLOAD_MAX {
        Some(SYSTEM_MANAGER_PAYLOAD_MAX)
    } else {
        Some(payload_bytes)
    }
}

/// The largest index of the system manager's request handler table. The
/// table holds 62 entries [OFW-4.93 lv1 a:0x46FD60-0x46FF4C].
pub const SYSTEM_MANAGER_HANDLER_INDEX_MAX: u8 = 0x3D;

/// The index that the system manager's receive function takes for a
/// service id whose low byte is above [`SYSTEM_MANAGER_HANDLER_INDEX_MAX`]
/// [OFW-4.93 lv1 a:0x44CE18-0x44CE20].
///
/// Linux names service id 6 `PS3_SM_SERVICE_ID_REQUEST_ERROR`.
pub const SYSTEM_MANAGER_REQUEST_ERROR_INDEX: u8 = 6;

/// The index of the request handler that the system manager's receive
/// function calls for the service id `service`.
///
/// The function loads only the low byte of the service id, header byte 9
/// [OFW-4.93 lv1 a:0x44CE14]. A byte above
/// [`SYSTEM_MANAGER_HANDLER_INDEX_MAX`] becomes
/// [`SYSTEM_MANAGER_REQUEST_ERROR_INDEX`] [OFW-4.93 lv1 a:0x44CE18-0x44CE20].
pub const fn system_manager_handler_index(service: u16) -> u8 {
    let low = service.to_be_bytes()[1];
    if low > SYSTEM_MANAGER_HANDLER_INDEX_MAX {
        SYSTEM_MANAGER_REQUEST_ERROR_INDEX
    } else {
        low
    }
}

/// The service id of the 4.93 GameOS kernel's first request to the
/// system manager in CellGov's boot.
///
/// The kernel's function stores it at +8 of the header
/// [OFW-4.93 lv2_kernel a:0x80000000002646fc]
/// [OFW-4.93 lv2_kernel a:0x800000000026470c]. Entry 9 of the receive
/// function's table holds its handler; see [`crate::lv1::uart`].
pub const GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE: u16 = 9;

/// The header that the system manager writes to its port before a reply
/// of the service id `service`, with `payload_bytes` payload bytes and
/// the tag `tag`.
///
/// The send function gives the header builder the low byte of its service
/// id, its tag, and its payload count plus its r7
/// [OFW-4.93 lv1 a:0x44C4CC-0x44C510]. The builder sets these fields
/// [OFW-4.93 lv1 a:0x44BA78-0x44BAA8]:
/// - the version 1 at +0 and the size 16 at +1;
/// - the count as a word at +4;
/// - the service id as a halfword at +8;
/// - the tag as a word at +12;
/// - 0 in each other byte.
///
/// The send function then writes the header and the payload with one
/// write [OFW-4.93 lv1 a:0x44C514-0x44C558].
pub const fn system_manager_reply_header(
    service: u8,
    payload_bytes: u32,
    tag: u32,
) -> [u8; SYSTEM_MANAGER_HEADER_BYTES as usize] {
    let n = payload_bytes.to_be_bytes();
    let t = tag.to_be_bytes();
    [
        1, 0x10, 0, 0, n[0], n[1], n[2], n[3], 0, service, 0, 0, t[0], t[1], t[2], t[3],
    ]
}

/// The offset of the payload byte that the handler of service id 9
/// branches on [OFW-4.93 lv1 a:0x44DEA0].
pub const SYSTEM_MANAGER_SELECTOR_OFFSET: usize = 1;

/// The console capture of the reply fields that the 4.93 kernel kept from
/// the system manager's replies of service ids 0x0B and 0x0A.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the kernel addresses.
pub const SYSTEM_MANAGER_REPLY_CAPTURE: &str = "tests/micro/lv2_system_manager_reply";

/// The most bytes that the system manager's init keeps for the reply of
/// service id 0x0A.
///
/// The init sets the count to byte 5 of the block at vaddr 0xC000D35B
/// only when that byte is not above 128 [OFW-4.93 lv1 a:0x44CFE0]
/// [OFW-4.93 lv1 a:0x4527D0-0x452800].
pub const SYSTEM_MANAGER_INIT_STORED_MAX: u64 = 128;

/// The system manager's reply to one request of the GameOS kernel, at one
/// firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsSystemManagerReply {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The index of the request handler; see
    /// [`system_manager_handler_index`].
    pub handler: u8,
    /// The payload byte at [`SYSTEM_MANAGER_SELECTOR_OFFSET`].
    pub selector: u8,
    /// The service id of the reply.
    pub reply_service: u8,
    /// The payload bytes of the reply.
    pub payload: &'static [u8],
    /// The bytes that the handler writes after the payload.
    pub trailing: &'static [u8],
}

impl GameOsSystemManagerReply {
    /// The payload count in the reply's header.
    ///
    /// The send function adds its r7 to its payload count
    /// [OFW-4.93 lv1 a:0x44C4CC]. A handler that writes bytes after the
    /// payload gives their count in r7 [OFW-4.93 lv1 a:0x44DF50].
    pub const fn header_count(&self) -> usize {
        self.payload.len() + self.trailing.len()
    }

    /// The bytes that the system manager writes to its port for this reply
    /// to a request with the tag `tag`, or `None` when the header count
    /// does not fit in a word.
    ///
    /// The bytes are the header of [`system_manager_reply_header`], the
    /// payload, and then the trailing bytes. The handler writes the
    /// trailing bytes after the send function returns
    /// [OFW-4.93 lv1 a:0x44DF64-0x44DFC8].
    pub fn written_bytes(&self, tag: u32) -> Option<Vec<u8>> {
        let count = u32::try_from(self.header_count()).ok()?;
        let head = system_manager_reply_header(self.reply_service, count, tag);
        Some([&head[..], self.payload, self.trailing].concat())
    }
}

/// Each system manager reply that a source gives.
///
/// ## Service id 0x0B
///
/// The handler of service id 9 replies to payload byte 1 = 2 with service
/// id 0x0B, its tag and 16 payload bytes
/// [OFW-4.93 lv1 a:0x44DEA8] [OFW-4.93 lv1 a:0x44DF10-0x44DF24]. Its r7
/// is 0, so the header counts 16. The function at
/// [OFW-4.93 lv1 a:0x44BAF4-0x44BB14] builds these payload bytes:
/// - byte 0 is 2;
/// - bytes 1 and 2 are bytes 2 and 3 at the system manager's vaddr
///   0xC000D35B [OFW-4.93 lv1 a:0x45242C-0x452440];
/// - byte 3 is 0;
/// - bytes 4 to 7 are the word at vaddr 0xC000D2D8
///   [OFW-4.93 lv1 a:0x44DEF4] [OFW-4.93 lv1 a:0x44DF08];
/// - bytes 8 to 15 are the doubleword at +96 of the partition's object
///   [OFW-4.93 lv1 a:0x44DEE0-0x44DEF0].
///
/// The capture [`SYSTEM_MANAGER_REPLY_CAPTURE`] gives bytes 1, 2 and 4 to
/// 15. No image read gives each of them:
/// - The system controller setup stores the word at 0xC000D2D8
///   [OFW-4.93 lv1 a:0x453DE4-0x453DF8]. The word is bytes 4 to 7 of a
///   reply of the system controller on `/dev/sc1`
///   [OFW-4.93 lv1 a:0x4547E8-0x454810] [OFW-4.93 lv1 a:0x453C38-0x453C4C].
/// - The partition's constructor sets +96 to 7 when a byte of system
///   manager state is 0 [OFW-4.93 lv1 a:0x4501BC-0x4501FC].
/// - Bytes 1 and 2 hold the byte at +24 of that object and a byte of
///   system manager state [OFW-4.93 lv1 a:0x44F3D0-0x44F3DC]. The
///   constructor sets +24 to 1 [OFW-4.93 lv1 a:0x4501D4].
///
/// ## Service id 0x0A
///
/// The handler of service id 9 replies to payload byte 1 = 1 with service
/// id 0x0A, its tag and 8 payload bytes
/// [OFW-4.93 lv1 a:0x44DF2C-0x44DF60]:
/// - byte 0 is the request's payload byte 1 [OFW-4.93 lv1 a:0x44DF58];
/// - byte 1 is 0 [OFW-4.93 lv1 a:0x44DF38];
/// - bytes 2 and 3 are the low halfword of the count at vaddr
///   0xC000D1A0 [OFW-4.93 lv1 a:0x44DF44] [OFW-4.93 lv1 a:0x44DF54];
/// - bytes 4 to 7 are 0 [OFW-4.93 lv1 a:0x44DF5C].
///
/// Its r7 is the count. It then writes the count's bytes from the buffer
/// whose address is at vaddr 0xC000D1A8 [OFW-4.93 lv1 a:0x44DF64-0x44DFC8].
/// The init stores the address of a 1536-byte buffer there
/// [OFW-4.93 lv1 a:0x44CFC0-0x44CFF8]. The TOC slots r2 - 29456 and
/// r2 - 29464 hold these vaddrs [OFW-4.93 lv1 a:0x47D968-0x47D974].
///
/// CellGov's boot is the first GameOS boot after lv1 starts, and the
/// count is 0 at that boot:
/// - The init calls the system controller setup and then stores the count
///   [OFW-4.93 lv1 a:0x44B164-0x44B170]. The setup clears the 256 bytes
///   of the block at vaddr 0xC000D35B. It fills them only when bit 0 of
///   the word at vaddr 0xC000D2D8 is set
///   [OFW-4.93 lv1 a:0x453E40-0x453E64]. The capture gives that word as
///   4, so byte 5 of the block is 0.
/// - The init sets the count to byte 5; see
///   [`SYSTEM_MANAGER_INIT_STORED_MAX`].
/// - Only the handler of a request of service id 0x0A sets another count
///   [OFW-4.93 lv1 a:0x44D448-0x44D470]. The kernel sends that request
///   from `sys_sm_shutdown` [OFW-4.93 lv2_kernel a:0x800000000000aae8].
///   No row answers it, so no request in CellGov's boot sets the count.
/// - The reply takes each byte that it writes from the count
///   [OFW-4.93 lv1 a:0x44DFA4-0x44DFAC], so a later reply also counts 0.
///
/// The capture gives the count that the console's kernel kept as 1536.
/// That count is above the init's largest, so it came from a
/// `sys_sm_shutdown` call of an earlier GameOS boot.
pub const GAME_OS_SYSTEM_MANAGER_REPLIES: &[GameOsSystemManagerReply] = &[
    GameOsSystemManagerReply {
        firmware: "4.93",
        handler: 9,
        selector: 2,
        reply_service: 0x0B,
        payload: &[2, 1, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 7],
        trailing: &[],
    },
    GameOsSystemManagerReply {
        firmware: "4.93",
        handler: 9,
        selector: 1,
        reply_service: 0x0A,
        payload: &[1, 0, 0, 0, 0, 0, 0, 0],
        trailing: &[],
    },
];

/// The reply to a request of service id `service` whose payload byte at
/// [`SYSTEM_MANAGER_SELECTOR_OFFSET`] is `selector`, at the firmware
/// version `firmware`, or `None` when no source gives it.
///
/// The receive function selects the handler by
/// [`system_manager_handler_index`].
pub fn game_os_system_manager_reply(
    firmware: &str,
    service: u16,
    selector: u8,
) -> Option<&'static GameOsSystemManagerReply> {
    let handler = system_manager_handler_index(service);
    GAME_OS_SYSTEM_MANAGER_REPLIES
        .iter()
        .find(|row| row.firmware == firmware && row.handler == handler && row.selector == selector)
}

#[cfg(test)]
#[path = "tests/uart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/uart_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/uart_param_tests.rs"]
mod param_tests;

#[cfg(test)]
#[path = "tests/uart_param_console_tests.rs"]
mod param_console_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_tests.rs"]
mod set_param_tests;

#[cfg(test)]
#[path = "tests/uart_set_param_console_tests.rs"]
mod set_param_console_tests;

#[cfg(test)]
#[path = "tests/uart_status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "tests/uart_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "tests/uart_service_port_tests.rs"]
mod service_port_tests;

#[cfg(test)]
#[path = "tests/uart_write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "tests/uart_reply_tests.rs"]
mod reply_tests;

#[cfg(test)]
#[path = "tests/uart_reply_console_tests.rs"]
mod reply_console_tests;

#[cfg(test)]
#[path = "tests/uart_random_request_tests.rs"]
mod random_request_tests;

#[cfg(test)]
#[path = "tests/uart_shadow_tests.rs"]
mod shadow_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_tests.rs"]
mod system_manager_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_reply_tests.rs"]
mod system_manager_reply_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_reply_console_tests.rs"]
mod system_manager_reply_console_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_stored_reply_tests.rs"]
mod system_manager_stored_reply_tests;

#[cfg(test)]
#[path = "tests/uart_system_manager_stored_reply_console_tests.rs"]
mod system_manager_stored_reply_console_tests;
