# Hypervisor

lv1 is the PS3 hypervisor. A partition's kernel calls it with
hypercalls, and lv1's own processes serve some of the kernel's
requests. CellGov's hypervisor, `cellgov_lv1`, answers the GameOS
kernel's hypercalls in lv1's place, from the facts in
`cellgov_ps3_abi::lv1`. It does not run lv1.

This document holds the firmware readings behind those facts: how each
value was found in lv1 and in the kernel. The code keeps each value and
the facts specific to it. Each fact here carries its firmware tag, and
`cellgov dev disasm` reads the window that the tag names. The
repository holds no byte of lv1, only these facts.

These conventions apply to each section:

- The readings are of the 4.93 `lv1.self` and `lv2_kernel.self`,
  decrypted offline.
- `+N` is the byte offset N in an object.
- An r3 "with rN marked" has the bit of argument register rN in its
  high word: bit 32 for r3, bit 33 for r4 and bit 34 for r5.
- "The vector's entry" is the entry of lv1's hypercall vector, which
  zeroes the return slots before the handler runs.

## Virtual UART

The facts live in `cellgov_ps3_abi::lv1::uart`. The sections follow a
port from its configuration, through its parameters and its traffic, to
the two lv1 services that hold the peers of the GameOS kernel's service
ports.

### Return slots

The handler of `configure_virtual_uart_irq` (hypercall 166)
[OFW-4.93 lv1 a:0x2F2810] reads r3 and no other register
[OFW-4.93 lv1 a:0x2F2824]. r3 is the logical partition address of a
32-byte bitmap. The handler returns the r3 that it builds in r31
[OFW-4.93 lv1 a:0x2F2A48]. Only a call that succeeds writes the return
slot of r4; see `lv1::event_port` for that slot. The vector's entry
zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0, and r4
returns 0 for a call that fails.

### The checks of configure_virtual_uart_irq

The handler checks r3 in this order, and the first failure is the
answer:

1. r3 is not above `VIRTUAL_UART_IRQ_ADDRESS_MAX`.
2. A memory region holds r3. The handler looks up the region with the
   function [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F28BC-0x2F28F8],
   as `configure_irq_state_bitmap` does; see `lv1::interrupt`.
3. The region's class at +72 is the class in the TOC slot at r2 - 5872
   [OFW-4.93 lv1 a:0x2F28FC-0x2F2910]. That slot and the slot that
   `configure_irq_state_bitmap` checks, r2 - 7048, hold the same class
   [OFW-4.93 lv1 a:0x358948] [OFW-4.93 lv1 a:0x3584B0].
4. The region holds `VIRTUAL_UART_IRQ_BITMAP_BYTES` bytes from r3.
5. The bitmap and its next byte are in one page of the region.
6. The region has a page array at +176 [OFW-4.93 lv1 a:0x2F2948-0x2F2950].

`CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3` and the constants after it
give the r3 of each check.

### The configure step

The handler translates r3 to a real address through the region's page
array [OFW-4.93 lv1 a:0x2F2954-0x2F2984]. It then gives the function
[OFW-4.93 lv1 a:0x2F13FC] the partition's virtual UART interrupt
object, at +0x120 of the partition, and the real address
[OFW-4.93 lv1 a:0x2F2958-0x2F298C]. That function does these steps in
this order:

1. It returns 5 when the object holds an outlet at +16
   [OFW-4.93 lv1 a:0x2F1474-0x2F1480]. The handler maps 5 to
   `CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3`.
2. It allocates a 152-byte object and constructs it with the function
   [OFW-4.93 lv1 a:0x2DF6B8] [OFW-4.93 lv1 a:0x2F1484-0x2F14B0]. That
   function has the instructions and the TOC slots of the event receive
   port constructor [OFW-4.93 lv1 a:0x2DF60C-0x2DF6B4]
   [OFW-4.93 lv1 a:0x2DF6B8-0x2DF760]. So the outlet takes the first
   free id of the partition's object table, and has the port class and
   no connection; see `lv1::event_port`.
3. It checks that the region is live and is the caller's
   [OFW-4.93 lv1 a:0x2F1528-0x2F1568] [OFW-4.93 lv1 a:0x2F1790-0x2F17DC].
4. It keeps the real address at +48, the region at +24 and the outlet
   at +16 of the object
   [OFW-4.93 lv1 a:0x2F15C4-0x2F15CC] [OFW-4.93 lv1 a:0x2F15E8-0x2F15EC].
5. It copies the object's 32-byte shadow at +56 to the bitmap
   [OFW-4.93 lv1 a:0x2F15FC-0x2F1620].
6. It puts the outlet in the slot of the object table that the
   outlet's id at +72 names [OFW-4.93 lv1 a:0x2F1624-0x2F16F0].
7. It gives the outlet to the event function [OFW-4.93 lv1 a:0x2DF808]
   when the count of set shadow bits at +2200 is above 0
   [OFW-4.93 lv1 a:0x2F16F4-0x2F1708].

The handler then stores the outlet's id in the return slot of r4, and
returns 0 [OFW-4.93 lv1 a:0x2F2A6C-0x2F2A8C].

Past the checks, these answers depend on lv1's own state:

- The live check of step 3 returns `CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3`
  [OFW-4.93 lv1 a:0x2F17C8-0x2F17D8] [OFW-4.93 lv1 a:0x2F29BC-0x2F29D4].
- A failed allocation returns `LV1_RESOURCE_SHORTAGE` (-2),
  zero-extended [OFW-4.93 lv1 a:0x2F1494-0x2F14A0]
  [OFW-4.93 lv1 a:0x2F2AA0-0x2F2AB4].

The outlet passes each check of `connect_irq_plug_ext`, as an irq
outlet of a logical SPE does; see `lv1::spe`. The constructor
stores the word in the TOC slot at r2 - 6032 at +136 of the outlet
[OFW-4.93 lv1 a:0x2F14AC] [OFW-4.93 lv1 a:0x2DF710]. That word
[OFW-4.93 lv1 a:0x3588A8] is not the word of a port
[OFW-4.93 lv1 a:0x358160]. So `set_spe_transition_notifier` refuses the
outlet, as it refuses an outlet of a logical SPE.

### The kernel's outlet

The 4.93 kernel makes its `configure_virtual_uart_irq` call in the
function [OFW-4.93 lv2_kernel a:0x8000000000017ad0], and calls that
function again until it returns 0
[OFW-4.93 lv2_kernel a:0x8000000000017cac-0x8000000000017cb8]. The
partition's other objects decide the outlet id, so the console capture
in `GAME_OS_VIRTUAL_UART_IRQ_CALLS` gives it.

### The shadow

The object's init function [OFW-4.93 lv1 a:0x2F1258] sets these
fields:

- +16, the outlet, is 0 [OFW-4.93 lv1 a:0x2F12AC].
- +48 holds the address of the object's own doublewords at +88
  [OFW-4.93 lv1 a:0x2F1270] [OFW-4.93 lv1 a:0x2F12BC].
- +56 to +87, the shadow, are 0 [OFW-4.93 lv1 a:0x2F130C-0x2F1318].
- +2200, the count of set shadow bits, is 0 [OFW-4.93 lv1 a:0x2F1308].

The raise step sets a shadow bit, adds 1 to the count, and stores the
shadow doubleword at the address that +48 holds
[OFW-4.93 lv1 a:0x2F225C-0x2F2278]. It does this only when the
endpoint's interrupt mask at +152 holds a bit of the event
[OFW-4.93 lv1 a:0x2F221C-0x2F2238]. The endpoint constructor sets the
mask to 0 [OFW-4.93 lv1 a:0x2F18DC]. The mask setter
[OFW-4.93 lv1 a:0x2F3F40-0x2F3F8C] is a step of the worker
[OFW-4.93 lv1 a:0x2F3C0C] of `set_virtual_uart_param` (hypercall 164),
which its handler calls [OFW-4.93 lv1 a:0x2F419C]. So the shadow and
the count stay 0 until the partition sets an endpoint's mask to a
value other than 0; see the section on the interrupt status.

### The parameters of a port

The handler of `get_virtual_uart_param` (hypercall 165)
[OFW-4.93 lv1 a:0x2F3A68] keeps r3 and r4
[OFW-4.93 lv1 a:0x2F3A78] [OFW-4.93 lv1 a:0x2F3A90]. It zeroes a stack
slot, and gives r3, r4 and the slot to the worker
[OFW-4.93 lv1 a:0x2F3AF0-0x2F3B04]. It then stores the slot in the
return slot of r4 [OFW-4.93 lv1 a:0x2F3B08-0x2F3B18], and returns the
worker's r3 [OFW-4.93 lv1 a:0x2F3B7C]. The worker writes the slot
only when it returns 0, and the vector's entry zeroes the return slots
[OFW-4.93 lv1 a:0x0C08]. So r4 returns 0 for a call that fails, and r5
to r10 return 0.

The worker [OFW-4.93 lv1 a:0x2F34E0] does these steps in this order:

1. It refuses r3 above `VIRTUAL_UART_PORT_MAX`
   [OFW-4.93 lv1 a:0x2F34E0] [OFW-4.93 lv1 a:0x2F34F8].
2. It reads the port's endpoint from the slot of r3 in the table at
   +152 of the virtual UART object, at +0x120 of the partition
   [OFW-4.93 lv1 a:0x2F34FC-0x2F3580]. It refuses a slot with no
   endpoint [OFW-4.93 lv1 a:0x2F3584-0x2F35C0].
3. It refuses r4 above `VIRTUAL_UART_PARAM_MAX`
   [OFW-4.93 lv1 a:0x2F35C4-0x2F35D8].
4. It reads the parameter through a table of eight entries
   [OFW-4.93 lv1 a:0x2F35F0-0x2F3628], stores it in the slot, and
   returns 0. The entry for 7 computes the port's interrupt status;
   see `VIRTUAL_UART_PARAM_INTERRUPT_STATUS`.

The endpoint's init function stores its buffer size at +112
[OFW-4.93 lv1 a:0x2EF650] [OFW-4.93 lv1 a:0x2EF68C].
`set_virtual_uart_param` sets only the parameters 0 to 2 and refuses
the others [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08]
[OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04]. So no hypercall changes a buffer
size. lv1 makes the endpoints when it makes the partition, so no image
read gives their sizes; see `GAME_OS_VIRTUAL_UART_PORTS`.

lv1's own code can change the two sizes while the partition runs:

- The endpoint holds its peer in a weak reference at +56, whose target
  is at +72 [OFW-4.93 lv1 a:0x2EF734-0x2EF744]. A function links two
  endpoints as peers [OFW-4.93 lv1 a:0x2EFE6C-0x2EFEC0], and another
  unlinks them [OFW-4.93 lv1 a:0x2EFB50-0x2EFB60]. lv1's service code
  calls the link function [OFW-4.93 lv1 a:0x29A218]
  [OFW-4.93 lv1 a:0x2AA320] [OFW-4.93 lv1 a:0x2AA420]
  [OFW-4.93 lv1 a:0x2F5904].
- The word at +8 is the reference count, in steps of 4
  [OFW-4.93 lv1 a:0x2BED58-0x2BED6C] [OFW-4.93 lv1 a:0x2BEE78-0x2BEE8C].
  So parameter 5 gives 0 once the peer has no reference.
- The port function puts a new endpoint in the port's slot and
  releases the old one [OFW-4.93 lv1 a:0x2F1A0C-0x2F1A1C]. lv1's service
  code calls it [OFW-4.93 lv1 a:0x29A390] [OFW-4.93 lv1 a:0x2A99FC].

No image read shows when that code runs. So the capture gives the sizes
only at the kernel's own calls; see `GAME_OS_VIRTUAL_UART_PARAM_CALLS`.

The 4.93 kernel's open function reads both sizes with hypercall 165
[OFW-4.93 lv2_kernel a:0x8000000000017708-0x8000000000017750]. It stores
them in its port record only after r3 = 0
[OFW-4.93 lv2_kernel a:0x8000000000017740]
[OFW-4.93 lv2_kernel a:0x800000000001775c]. The capture
`VIRTUAL_UART_PORT_CAPTURE` reads them from those records.

### Setting a parameter of a port

The handler of `set_virtual_uart_param` (hypercall 164)
[OFW-4.93 lv1 a:0x2F4100] gives r3, r4
and r5 to the worker [OFW-4.93 lv1 a:0x2F4190-0x2F419C]. It returns
the worker's r3 [OFW-4.93 lv1 a:0x2F41A4] [OFW-4.93 lv1 a:0x2F4208],
and writes no return slot. The vector's entry zeroes the return slots
[OFW-4.93 lv1 a:0x0C08], so r4 to r10 return 0.

The worker [OFW-4.93 lv1 a:0x2F3C0C] does these steps in this order:

1. It refuses r3 above `VIRTUAL_UART_PORT_MAX`
   [OFW-4.93 lv1 a:0x2F3C0C] [OFW-4.93 lv1 a:0x2F3C30].
2. It reads the port's endpoint from the slot of r3 in the table at
   +152 of the virtual UART object [OFW-4.93 lv1 a:0x2F3C34-0x2F3CB8].
   It refuses a slot with no endpoint [OFW-4.93 lv1 a:0x2F3CF4-0x2F3CF8].
3. It refuses r4 above `VIRTUAL_UART_SET_PARAM_MAX`
   [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08] [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04].
4. For the mask, it refuses r5 above `VIRTUAL_UART_INTERRUPT_MASK_MAX`.
5. It stores r5 in the endpoint's field of r4; see
   `VIRTUAL_UART_PARAM_TX_TRIGGER` for the fields.
6. It calls each listener in the list at +48 of the endpoint
   [OFW-4.93 lv1 a:0x2F3D70-0x2F3DC4], and returns 0
   [OFW-4.93 lv1 a:0x2F3DC8-0x2F3DCC].

The endpoint constructor sets the three fields to 0
[OFW-4.93 lv1 a:0x2F1864] [OFW-4.93 lv1 a:0x2F18D4-0x2F18DC]. The
image read has no other store to the three fields of an endpoint.
`get_virtual_uart_param` reads them for the parameters 0 to 2
[OFW-4.93 lv1 a:0x2F3670] [OFW-4.93 lv1 a:0x2F36F4]
[OFW-4.93 lv1 a:0x2F3748]. The port function can put a new endpoint in
the port's slot, as the section on the parameters of a port gives. So
the three values of a port can return to 0 at a time that no image
read shows.

### The interrupt status

The port function adds a listener to the endpoint that it puts in a
port's slot [OFW-4.93 lv1 a:0x2F1A50-0x2F1A70]. The listener holds the
virtual UART object and the status function
[OFW-4.93 lv1 a:0x2F0F18-0x2F0F94] [OFW-4.93 lv1 a:0x3588C0]
[OFW-4.93 lv1 a:0x349908]. The method that the worker calls
[OFW-4.93 lv1 a:0x3625F0-0x362604] [OFW-4.93 lv1 a:0x3495A8] calls the
status function with the object and the endpoint
[OFW-4.93 lv1 a:0x2F0AF4-0x2F0B38].

The list at +48 of an endpoint holds only this listener:

- The port function's call is the only call of the listener's builder
  in the image read, and no word of the image holds the builder's
  descriptor [OFW-4.93 lv1 a:0x349728].
- The builder claims the slot at +160 of the endpoint. When the slot
  holds a listener, it removes the new listener from the list again
  [OFW-4.93 lv1 a:0x2F0FBC-0x2F1058].
- The other builders of the same form add their listener to an object
  that they make. That object is no endpoint
  [OFW-4.93 lv1 a:0x209E5C-0x209ED4] [OFW-4.93 lv1 a:0x22E0B4-0x22E12C]
  [OFW-4.93 lv1 a:0x2F55D8-0x2F5668].

The status function [OFW-4.93 lv1 a:0x2F20E4] acts only when the
port's slot holds the endpoint [OFW-4.93 lv1 a:0x2F2128-0x2F2140]. It
sets these bits of the status:

- 1 when the endpoint has a peer with a reference, and the doubleword
  at +120 of the peer is not above the transmit trigger
  [OFW-4.93 lv1 a:0x2F21E0-0x2F2204] [OFW-4.93 lv1 a:0x2F2380-0x2F2384];
- 2 when the buffer size less the doubleword at +120 of the endpoint
  is not above the receive trigger
  [OFW-4.93 lv1 a:0x2F2208-0x2F2210] [OFW-4.93 lv1 a:0x2F233C-0x2F2348];
- 4 when the endpoint has no peer with a reference
  [OFW-4.93 lv1 a:0x2F21B0-0x2F21C8] [OFW-4.93 lv1 a:0x2F21E8]
  [OFW-4.93 lv1 a:0x2F23A4];
- 8 when the endpoint has a peer with a reference
  [OFW-4.93 lv1 a:0x2F21F4-0x2F21FC]. No mask that lv1 accepts holds
  this bit.

The function computes the status after each call of
`set_virtual_uart_param` that succeeds, the mask call included
[OFW-4.93 lv1 a:0x2F3FB0-0x2F4008]. It then does one of these steps:

- When the byte at +128 of the endpoint is 0, and the status and the
  mask share a bit [OFW-4.93 lv1 a:0x2F221C-0x2F224C], it sets the
  port's shadow bit, as the section on the shadow gives
  [OFW-4.93 lv1 a:0x2F217C-0x2F2184] [OFW-4.93 lv1 a:0x2F221C-0x2F2278].
  It gives the outlet to the event function when the count becomes 1
  [OFW-4.93 lv1 a:0x2F227C-0x2F2280] [OFW-4.93 lv1 a:0x2F2394-0x2F239C].
- Otherwise, when the port's shadow bit is set, it clears the bit
  [OFW-4.93 lv1 a:0x2F22DC-0x2F2334].

So a call that leaves the mask at 0 sets no shadow bit, and changes no
byte while the shadow is 0. A call that leaves a mask other than 0 can
raise the port's interrupt. Whether it does depends on the peer and on
the doublewords at +120, which change with the port's traffic;
`virtual_uart_status` computes the status from them. The kernel sets
such a mask on each of its ports; see `VIRTUAL_UART_PORT_MASK_CAPTURE`.

### Reading a port

The handler of `read_virtual_uart` (hypercall 162)
[OFW-4.93 lv1 a:0x2F429C] keeps r3, r4
and r5 [OFW-4.93 lv1 a:0x2F42B0] [OFW-4.93 lv1 a:0x2F42D8-0x2F42DC].
It does these steps in this order:

1. It refuses r3 above `VIRTUAL_UART_PORT_MAX`
   [OFW-4.93 lv1 a:0x2F433C-0x2F4340].
2. It refuses r4 above `VIRTUAL_UART_READ_ADDRESS_MAX`.
3. It reads the port's endpoint from the slot of r3 in the table at
   +152 of the virtual UART object [OFW-4.93 lv1 a:0x2F4354-0x2F43D4].
   It refuses a slot with no endpoint [OFW-4.93 lv1 a:0x2F4410-0x2F4414].
4. It looks up the memory region of r4 with the function
   [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F4418-0x2F4444]. It
   checks the region's class against the TOC slot at r2 - 5872, as
   `configure_virtual_uart_irq` does [OFW-4.93 lv1 a:0x2F4448-0x2F4458].
5. It refuses a region with fewer than r5 bytes from r4 to its end
   [OFW-4.93 lv1 a:0x2F445C-0x2F446C], and a region with no page array
   at +176 [OFW-4.93 lv1 a:0x2F4470-0x2F4478].
6. For each page of the buffer, it gives the endpoint, the page's real
   address and the bytes left in the page to the read function
   [OFW-4.93 lv1 a:0x2F447C-0x2F44F4]. It stops after a read of 0
   bytes, or when no byte is left [OFW-4.93 lv1 a:0x2F4500-0x2F4528].
7. It stores the sum of the bytes in the return slot of r4, and returns
   0 [OFW-4.93 lv1 a:0x2F452C-0x2F4538] [OFW-4.93 lv1 a:0x2F45A0].

A refusal skips that store [OFW-4.93 lv1 a:0x2F4604-0x2F4614], and the
vector's entry zeroes the return slots [OFW-4.93 lv1 a:0x0C08]. So r4
returns 0 for a call that fails, and r5 to r10 return 0. See
`READ_VIRTUAL_UART_NO_PORT_R3` for the r3 of each check.

The read function [OFW-4.93 lv1 a:0x2F06C0] reads the endpoint's
receive buffer:

- It returns 7 when the byte at +128 of the endpoint is not 0
  [OFW-4.93 lv1 a:0x2F0740-0x2F074C].
- It copies the lesser of the bytes asked for and the doubleword at
  +120, the count of bytes that the buffer holds
  [OFW-4.93 lv1 a:0x2F0750-0x2F0770]. It copies from the read pointer
  at +96, which wraps in the buffer at +104 of the size at +112
  [OFW-4.93 lv1 a:0x2F06D8-0x2F0710] [OFW-4.93 lv1 a:0x2F0780-0x2F07F0].
- After it copies one or more bytes, it stores the new read pointer and
  count [OFW-4.93 lv1 a:0x2F07FC-0x2F0810]. It then calls the listeners
  of the endpoint and of its peer [OFW-4.93 lv1 a:0x2F0830-0x2F0940],
  which compute the interrupt status that the section above gives.
- When the buffer holds no byte and the endpoint has a peer with a
  reference, it returns 0 with 0 bytes read, and changes nothing
  [OFW-4.93 lv1 a:0x2F09F8-0x2F0A64] [OFW-4.93 lv1 a:0x2F0944-0x2F0948].
- When the buffer holds no byte and the endpoint has no such peer, the
  handler returns `LV1_WRONG_STATE` (-11), with r3 marked
  [OFW-4.93 lv1 a:0x2F0A68-0x2F0A70] [OFW-4.93 lv1 a:0x2F45E0-0x2F45F0]
  [OFW-4.93 lv1 a:0x2F464C-0x2F4658].

So the answer to a call that passes the checks depends on the bytes in
the port's receive buffer. lv1's endpoint write function
[OFW-4.93 lv1 a:0x2F0494] puts bytes in that buffer. These callers
reach it:

- `write_virtual_uart` (hypercall 163), for the bytes that the
  partition sends to the port's peer [OFW-4.93 lv1 a:0x2F2D90];
- four functions of lv1's own code
  [OFW-4.93 lv1 a:0x2F31B4] [OFW-4.93 lv1 a:0x2F53F0]
  [OFW-4.93 lv1 a:0x2F59DC] [OFW-4.93 lv1 a:0x2F775C].

One of these functions [OFW-4.93 lv1 a:0x2F30A0] serves a write whose
buffer lv1 translates through the function [OFW-4.93 lv1 a:0x29D074]
[OFW-4.93 lv1 a:0x2A9DB8] [OFW-4.93 lv1 a:0x2A9E20]. For most ports, no
image read shows when lv1's services make these writes, or which bytes
they write. The two service ports are the exception; see the section
on the service ports.

### Writing a port

The handler of `write_virtual_uart` (hypercall 163)
[OFW-4.93 lv1 a:0x2F2AF4] keeps r3, r4
and r5 [OFW-4.93 lv1 a:0x2F2B08] [OFW-4.93 lv1 a:0x2F2B2C-0x2F2B30].
It does these steps in this order:

1. It refuses r3 above `VIRTUAL_UART_PORT_MAX`
   [OFW-4.93 lv1 a:0x2F2B90-0x2F2B94].
2. It refuses r4 above `VIRTUAL_UART_WRITE_ADDRESS_MAX`.
3. It reads the port's endpoint from the slot of r3 in the table at
   +152 of the virtual UART object. It refuses a slot with no endpoint
   [OFW-4.93 lv1 a:0x2F2BA8-0x2F2C68].
4. It looks up the memory region of r4 with the function
   [OFW-4.93 lv1 a:0x2CE74C] [OFW-4.93 lv1 a:0x2F2C6C-0x2F2C98]. It
   makes the checks of `read_virtual_uart` on the region: the class
   [OFW-4.93 lv1 a:0x2F2C9C-0x2F2CAC], the r5 bytes from r4 to the end
   [OFW-4.93 lv1 a:0x2F2CB0-0x2F2CC0], and the page array at +176
   [OFW-4.93 lv1 a:0x2F2CC4-0x2F2CCC].
5. It refuses an endpoint with no peer that has a reference
   [OFW-4.93 lv1 a:0x2F2CF8-0x2F2D3C] [OFW-4.93 lv1 a:0x2F2F24-0x2F2F28].
6. For each page of the buffer, it gives the endpoint, the page's real
   address and the bytes left in the page to the write function
   [OFW-4.93 lv1 a:0x2F2D40-0x2F2D90]. It stops after a write of 0
   bytes, or when no byte is left [OFW-4.93 lv1 a:0x2F2D9C-0x2F2DF0].
7. It stores the sum of the bytes in the return slot of r4, and returns
   0 [OFW-4.93 lv1 a:0x2F2DF4-0x2F2E00] [OFW-4.93 lv1 a:0x2F2E68].

The return slot is the slot that `read_virtual_uart` writes
[OFW-4.93 lv1 a:0x2F452C-0x2F4538]. A refusal skips the store, and the
vector's entry zeroes the return slots [OFW-4.93 lv1 a:0x0C08]. So r4
returns 0 for a call that fails, and r5 to r10 return 0. A call with
r5 = 0 gives no byte to the write function, and returns r3 = 0 and
r4 = 0. See `WRITE_VIRTUAL_UART_NO_PORT_R3` for the r3 of each check.

Another failure of the write function before it writes a byte returns
`LV1_WRONG_STATE` for 0x9FE5F901, or 0xFFFF8000 for other values
[OFW-4.93 lv1 a:0x2F2EA4-0x2F2EC4]. A failure after it writes one or
more bytes returns 0 with the sum in r4
[OFW-4.93 lv1 a:0x2F2E94-0x2F2E98].

The write function [OFW-4.93 lv1 a:0x2F0494] writes to the receive
buffer of the endpoint's peer:

- When the endpoint has a peer with a reference, it gives the peer to
  the function [OFW-4.93 lv1 a:0x2F00F4]
  [OFW-4.93 lv1 a:0x2F04E4-0x2F0530].
- That function copies the lesser of the bytes asked for and the free
  bytes of the peer's buffer: the size at +112 less the count at +120
  [OFW-4.93 lv1 a:0x2F0290-0x2F02BC]. It copies to the write pointer at
  +88, which wraps in the buffer at +104
  [OFW-4.93 lv1 a:0x2F02C0-0x2F0318].
- After it copies one or more bytes, it stores the new write pointer
  and count [OFW-4.93 lv1 a:0x2F0324-0x2F0334]. It then calls the
  listeners of the peer [OFW-4.93 lv1 a:0x2F0358-0x2F03AC]. The write
  function then calls the listeners of the endpoint
  [OFW-4.93 lv1 a:0x2F0548-0x2F059C], which compute the interrupt
  status that the section on the interrupt status gives.
- When the endpoint has no such peer, or that function fails
  [OFW-4.93 lv1 a:0x2F0534-0x2F0538], it returns 7 when the byte at
  +128 of the endpoint is not 0, and 0x9FE5F901 when that byte is 0
  [OFW-4.93 lv1 a:0x2F062C-0x2F0644].

So a call that passes the checks writes all r5 bytes when the peer's
receive buffer has r5 free bytes. The service that holds the peer
reads them; see the next section.

### The service ports

lv1 carries a process archive [OFW-4.93 lv1 a:0x400000-0x4000E4]. Two of
its files hold the peer of a port of the GameOS partition:

- `sysmgr_ss.fself`, lv1's system manager, holds port 2. Its entry is
  the second [OFW-4.93 lv1 a:0x400014-0x40001C].
- `ss_server3.fself` holds port 10 for its dispatcher manager. Its
  entry is the eighth [OFW-4.93 lv1 a:0x40005C-0x400064].

Each service writes its port to reply to the partition's request, or on
an event that CellGov does not model. So each port's receive buffer is
empty at the partition's first read. `GAME_OS_SERVICE_PORTS` holds the
two ports. The next sections give the evidence for each service.

### The system manager

The system manager's boot function for a partition does these steps in
this order:

1. It calls the port function [OFW-4.93 lv1 a:0x44E990]. That function
   constructs port 2 of the partition's virtual UART with a buffer of
   2048 bytes, through process system call 0x10084
   [OFW-4.93 lv1 a:0x44CF04-0x44CF24] [OFW-4.93 lv1 a:0x464BC8-0x464BD8].
   It then opens the port's peer as a file, keeps the descriptor, and
   gives the descriptor a receive function
   [OFW-4.93 lv1 a:0x44CF58-0x44CF9C].
2. It activates the partition's logical PU [OFW-4.93 lv1 a:0x44E9B8].
3. It tells two other services that the boot is done
   [OFW-4.93 lv1 a:0x44E9E8] [OFW-4.93 lv1 a:0x44B26C-0x44B2AC]. That
   step does not use the descriptor.

The system manager writes the port only through the descriptor. Four
functions of its image read the descriptor:

- The send function writes a 16-byte header and a payload
  [OFW-4.93 lv1 a:0x44C4C4] [OFW-4.93 lv1 a:0x44C544-0x44C558].
- The receive function reads a header and a payload. It then calls the
  request handler of the header's service id through a table
  [OFW-4.93 lv1 a:0x44CD54] [OFW-4.93 lv1 a:0x44CD80-0x44CE44]
  [OFW-4.93 lv1 a:0x46FD60-0x46FF4C].
- One request handler writes more bytes after its reply
  [OFW-4.93 lv1 a:0x44DF70-0x44DF7C] [OFW-4.93 lv1 a:0x44DFEC-0x44DFF8].
- The teardown function closes the descriptor and destructs the port
  [OFW-4.93 lv1 a:0x44CE60-0x44CEC8].

Each call of the send function sends one of these messages:

- a reply, from a request handler that the receive function calls;
- a command, service id 3, when the system manager stops the partition
  [OFW-4.93 lv1 a:0x44CC10-0x44CC1C] [OFW-4.93 lv1 a:0x44F54C];
- an external event, service id 4, of a power or thermal event
  [OFW-4.93 lv1 a:0x44CC80-0x44CC8C] [OFW-4.93 lv1 a:0x452FE8-0x452FFC].

Linux `drivers/ps3/ps3-sys-manager.c` gives the same direction for each
of these service ids, in `enum ps3_sys_manager_service_id`. It gives the
codes 3, 4, 7 and 8 of the events in `enum ps3_sys_manager_event`.

So the system manager writes nothing to port 2 before the partition
sends a request, unless a shutdown or an external event comes first.
CellGov models neither. The system manager keeps the peer's descriptor
open until the teardown.

### The system manager's requests

The image `sysmgr_ss.fself` is fake-signed, and its two segments are
plaintext in lv1's image. Its SCE header gives a header size of 0x480
[OFW-4.93 lv1 a:0x4218C0-0x4218D4]. Its ELF program headers place the
segments at these addresses [OFW-4.93 lv1 a:0x421990-0x4219FC]:

- the code, at vaddr 0x80000000, at a:0x431D40;
- the data, at vaddr 0xC0000000, at a:0x471D40, with 0xCE88 bytes in the
  file of 0xDD58 bytes in memory.

The entry descriptor gives the TOC 0xC0012F40
[OFW-4.93 lv1 a:0x472FF0-0x472FFC].

The receive function [OFW-4.93 lv1 a:0x44CD54] does these steps in
this order:

1. It fills a header with the version 1, the size 16 and 0 in each
   other field [OFW-4.93 lv1 a:0x44CD60-0x44CD7C]
   [OFW-4.93 lv1 a:0x44BA78-0x44BAA8].
2. It reads `SYSTEM_MANAGER_HEADER_BYTES` bytes over the header with
   one read, process system call 9
   [OFW-4.93 lv1 a:0x44CD80-0x44CD94] [OFW-4.93 lv1 a:0x468DB0-0x468DBC].
   A read that returns another count ends the function with no handler
   [OFW-4.93 lv1 a:0x44CDB0-0x44CDB4].
3. It reads the payload bytes that
   `system_manager_payload_read` gives, in a loop of reads
   [OFW-4.93 lv1 a:0x44CDDC-0x44CE10].
4. It calls the request handler of `system_manager_handler_index`
   through the table [OFW-4.93 lv1 a:0x44CE14-0x44CE44]. The handler
   gets the payload and the tag; see `system_manager_header_tag`.

The table at TOC slot r2 - 29520 holds vaddr 0x8003E020, which is
a:0x46FD60 [OFW-4.93 lv1 a:0x47D930-0x47D934]. So no handler runs, and
no reply comes, until the partition sends the header and the payload
bytes that the receive function reads.

The 4.93 kernel's function [OFW-4.93 lv2_kernel a:0x80000000002646d8]
sends a request of service id 9 with 8 payload bytes in one write of
24 bytes [OFW-4.93 lv2_kernel a:0x80000000002646e0-0x800000000026471c].
It stores 1 in payload byte 0 and its r4 in payload byte 1. It stores
no other payload byte. In CellGov's boot, this request is the kernel's
first write to port 2, and its r4 is 2; see
`GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE`.

### The system manager's replies

Entry 9 of the table [OFW-4.93 lv1 a:0x46FDA8-0x46FDAC] holds the
descriptor of the handler [OFW-4.93 lv1 a:0x44DE84]
[OFW-4.93 lv1 a:0x477760-0x47776C]. The handler reads payload byte 1
[OFW-4.93 lv1 a:0x44DEA0-0x44DEC4]. For the value 2, it replies with
service id 0x0B and 16 payload bytes
[OFW-4.93 lv1 a:0x44DF0C-0x44DF24]. The function
[OFW-4.93 lv1 a:0x44BAF4-0x44BB14] builds these payload bytes:

- byte 0 is 2;
- bytes 1 and 2 are bytes 2 and 3 at vaddr 0xC000D35B
  [OFW-4.93 lv1 a:0x45242C-0x452440] [OFW-4.93 lv1 a:0x47E1B8-0x47E1BC];
- byte 3 is 0;
- bytes 4 to 7 are the word at vaddr 0xC000D2D8
  [OFW-4.93 lv1 a:0x44DEF4] [OFW-4.93 lv1 a:0x44DF08]
  [OFW-4.93 lv1 a:0x47DA48-0x47DA4C];
- bytes 8 to 15 are the doubleword at +96 of the current partition's
  state object, or 0 with no such object
  [OFW-4.93 lv1 a:0x44DEE0-0x44DEF0] [OFW-4.93 lv1 a:0x44F780-0x44F7A0].

Both vaddrs are past the 0xCE88 file bytes of the data segment. So the
system manager sets them while it runs, and no image read gives their
values. The 4.93 kernel keeps the reply's fields in its system manager
object, and the console capture `SYSTEM_MANAGER_REPLY_CAPTURE` of that
object gives bytes 1, 2 and 4 to 15; see `GAME_OS_SYSTEM_MANAGER_REPLIES`.
The image read gives where each of them comes from:

- The system controller setup stores the word at 0xC000D2D8
  [OFW-4.93 lv1 a:0x453DE4-0x453DF8]. The word is bytes 4 to 7 of a
  reply of the system controller on `/dev/sc1`
  [OFW-4.93 lv1 a:0x4547E8-0x454810] [OFW-4.93 lv1 a:0x453C38-0x453C4C].
- The partition's constructor sets +96 to 7 when a byte of system
  manager state is 0 [OFW-4.93 lv1 a:0x4501BC-0x4501FC].
- Bytes 1 and 2 hold the byte at +24 of that object and a byte of
  system manager state [OFW-4.93 lv1 a:0x44F3D0-0x44F3DC]. The
  constructor sets +24 to 1 [OFW-4.93 lv1 a:0x4501D4].

For payload byte 1 = 1, the handler replies with service id 0x0A and 8
payload bytes. It then writes the bytes that the system manager keeps
for the partition [OFW-4.93 lv1 a:0x44DF2C-0x44DFC8]. The payload
holds these fields:

- byte 0 is the request's payload byte 1 [OFW-4.93 lv1 a:0x44DF58];
- byte 1 is 0 [OFW-4.93 lv1 a:0x44DF38];
- bytes 2 and 3 are the low halfword of the count at vaddr
  0xC000D1A0 [OFW-4.93 lv1 a:0x44DF44] [OFW-4.93 lv1 a:0x44DF54];
- bytes 4 to 7 are 0 [OFW-4.93 lv1 a:0x44DF5C].

The handler's r7 is the count. It then writes the count's bytes from the buffer
whose address is at vaddr 0xC000D1A8 [OFW-4.93 lv1 a:0x44DF64-0x44DFC8].
The init stores the address of a 1536-byte buffer there
[OFW-4.93 lv1 a:0x44CFC0-0x44CFF8]. The TOC slots r2 - 29456 and
r2 - 29464 hold these vaddrs [OFW-4.93 lv1 a:0x47D968-0x47D974].

CellGov's boot is the first GameOS boot after lv1 starts, and the
count is 0 at that boot:

- The init calls the system controller setup and then stores the count
  [OFW-4.93 lv1 a:0x44B164-0x44B170]. The setup clears the 256 bytes
  of the block at vaddr 0xC000D35B. It fills them only when bit 0 of
  the word at vaddr 0xC000D2D8 is set
  [OFW-4.93 lv1 a:0x453E40-0x453E64]. The capture gives that word as
  4, so byte 5 of the block is 0.
- The init sets the count to byte 5; see
  `SYSTEM_MANAGER_INIT_STORED_MAX`.
- Only the handler of a request of service id 0x0A sets another count
  [OFW-4.93 lv1 a:0x44D448-0x44D470]. The kernel sends that request
  from `sys_sm_shutdown` [OFW-4.93 lv2_kernel a:0x800000000000aae8].
  No row answers it, so no request in CellGov's boot sets the count.
- The reply takes each byte that it writes from the count
  [OFW-4.93 lv1 a:0x44DFA4-0x44DFAC], so a later reply also counts 0.

The capture gives the count that the console's kernel kept as 1536.
That count is above the init's largest, so it came from a
`sys_sm_shutdown` call of an earlier GameOS boot.

### The dispatcher manager

The secure loader code of `ss_server3.fself` asks the dispatcher
manager for the partition's service port, port 10
[OFW-4.93 lv1 a:0x5B7E44-0x5B7E80]. The dispatcher manager constructs
the port with a buffer of 4096 bytes, through process system call
0x10084 [OFW-4.93 lv1 a:0x5BF1F0-0x5BF1FC] [OFW-4.93 lv1 a:0x5CA8FC-0x5CA90C].

The dispatcher manager relays requests and replies. Its receive loop
calls one of two functions for a socket that has data
[OFW-4.93 lv1 a:0x5BE5D4] [OFW-4.93 lv1 a:0x5BE5E8]:

- The port function reads the partition's packet and sends it to a
  socket that it connects to the packet's service
  [OFW-4.93 lv1 a:0x5BD888] [OFW-4.93 lv1 a:0x5BDC88]. In other cases,
  it replies to the port with a return value alone
  [OFW-4.93 lv1 a:0x5BD980] [OFW-4.93 lv1 a:0x5BDAFC].
- The service function reads a service's reply from such a socket, and
  writes it to the port [OFW-4.93 lv1 a:0x5BD3F0]
  [OFW-4.93 lv1 a:0x5BD524] [OFW-4.93 lv1 a:0x5BD540].

The service function's send function has no other caller. Two other
socket write functions have no direct caller: tables of function
descriptors hold them [OFW-4.93 lv1 a:0x5D356C]
[OFW-4.93 lv1 a:0x5D3674]. The image read does not show which objects
call them. Each write to the port that the image read shows is a reply
to a request. So the evidence supports this reading: the dispatcher
manager writes nothing to port 10 before the partition sends a request.

### The dispatcher manager's requests

The port function [OFW-4.93 lv1 a:0x5BD888] reads a request in this
order:

1. It reads a header of `DISPATCHER_HEADER_BYTES` bytes from the port
   [OFW-4.93 lv1 a:0x5BD8B4-0x5BD8E4]. The read function calls the
   loop [OFW-4.93 lv1 a:0x5BF300-0x5BF318] that reads until it holds
   each byte that it asks for [OFW-4.93 lv1 a:0x5BEFE0-0x5BF058].
2. It looks up the service id that the header gives; see
   `dispatcher_header_service`.
3. It reads or discards the request bytes that the header counts, and
   only then replies; see `dispatcher_header_request_bytes`.

Step 3 has three paths:

- For a service id that the lookup does not find, it discards the
  bytes [OFW-4.93 lv1 a:0x5BD930-0x5BD93C]. It then replies with a
  return value alone [OFW-4.93 lv1 a:0x5BD980].
- When it cannot get a buffer for the larger of the two sizes in the
  header, it discards the bytes [OFW-4.93 lv1 a:0x5BD994-0x5BD9D0]. It
  then replies with a return value alone [OFW-4.93 lv1 a:0x5BDAFC].
- Otherwise it reads the bytes [OFW-4.93 lv1 a:0x5BDA18-0x5BDA28], and
  sends them to a socket that it connects to the service.

The discard function reads pieces of at most 64 bytes until it holds
each byte [OFW-4.93 lv1 a:0x5BD6A4-0x5BD71C]. So the dispatcher manager
writes nothing to port 10 until the partition sends the header and each
request byte that it counts. The reply then comes from the service, or
has a return value that depends on the dispatcher manager's own state
[OFW-4.93 lv1 a:0x5BD948-0x5BD97C]. The section on the reply to the
kernel's first request gives the first reply that CellGov holds.

### The kernel's reads

At the 4.93 kernel's first read of a service port, the port's receive
buffer is empty, and the port has a peer with a reference. The capture
`VIRTUAL_UART_PORT_CAPTURE` gives each service port a transmit buffer
size other than 0 at the kernel's open. That value is 0 when the port
has no such peer; see `VIRTUAL_UART_PARAM_TX_BUFFER_SIZE`. So lv1's
read function returns 0 bytes, and the handler returns r3 = 0 and
r4 = 0.

The kernel reads each port through one function, which sends
hypercall 162 with the port from its port record
[OFW-4.93 lv2_kernel a:0x8000000000016b6c-0x8000000000016b7c]. Its
system manager loop opens port 2
[OFW-4.93 lv2_kernel a:0x8000000000264388], and reads a 16-byte message
header [OFW-4.93 lv2_kernel a:0x8000000000264bb0-0x8000000000264bc0].
CellGov's boot of the kernel makes its first read on port 10 before
its first `write_virtual_uart`. When lv1 reads 0 bytes, the kernel adds
the value 2 to the port's interrupt mask with hypercall 164, and waits
for the port's interrupt
[OFW-4.93 lv2_kernel a:0x8000000000016b9c-0x8000000000016be0]
[OFW-4.93 lv2_kernel a:0x8000000000016aa8-0x8000000000016ac8].

### The kernel's first request

The kernel writes each port through one function, which sends
hypercall 163 with the port from its port record
[OFW-4.93 lv2_kernel a:0x8000000000016fc8-0x8000000000016fd8]. In
CellGov's boot, the kernel's first two calls write port 10: a header,
then the request bytes that it counts. The request object of the
kernel holds the request id and the service id; see
`GAME_OS_FIRST_DISPATCHER_REQUEST`.

The kernel's dispatcher client sends a request in this form:

- The client's constructor sets the client's request count to 0 and
  its port to 10 [OFW-4.93 lv2_kernel a:0x8000000000212408]
  [OFW-4.93 lv2_kernel a:0x8000000000212428]
  [OFW-4.93 lv2_kernel a:0x8000000000212440].
- The send function gives the low word of the service id, the request
  size and the reply size to the header function
  [OFW-4.93 lv2_kernel a:0x80000000002131b0-0x8000000000213208]. For
  the class of the first request, both sizes are 0x38
  [OFW-4.93 lv2_kernel a:0x8000000000229c34].
- The header function puts the request count in the word at +0, then
  increments the count. It puts the service id, the request size and
  the reply size in the words at +4, +8 and +12
  [OFW-4.93 lv2_kernel a:0x80000000002121f4-0x80000000002121fc]
  [OFW-4.93 lv2_kernel a:0x800000000021221c-0x8000000000212230].
- It writes the 16 header bytes, then the request bytes
  [OFW-4.93 lv2_kernel a:0x8000000000212284-0x80000000002122ac].
- The request bytes start with the request id and the service id, as
  big-endian doublewords
  [OFW-4.93 lv2_kernel a:0x8000000000212e10-0x8000000000212eac].

So the client's first request has a header with tag 0, service
0x19000, 0x38 request bytes and 0x38 reply bytes.

### The reply to the kernel's first request

The dispatcher manager's map gives service 0x19000 the socket address
0x24 [OFW-4.93 lv1 a:0x5D7100-0x5D710C]. The builder of the map reads
that table [OFW-4.93 lv1 a:0x5BECDC-0x5BED50]. `ss_server2.fself` binds
a socket to that address and listens on it
[OFW-4.93 lv1 a:0x56C484-0x56C4A0] [OFW-4.93 lv1 a:0x58CF48-0x58CFBC].

`ss_server2` answers a request on one buffer, in this order:

1. The request class reads the request bytes from the buffer
   [OFW-4.93 lv1 a:0x58DF7C-0x58DFB4].
2. The class runs its handler [OFW-4.93 lv1 a:0x58DFB8-0x58DFD4].
3. The class writes its reply fields to the same buffer
   [OFW-4.93 lv1 a:0x58E044-0x58E07C].

For request 0x19002, the handler stores its worker's return value as
the status [OFW-4.93 lv1 a:0x56CC5C-0x56CC6C]. The worker builds the
16 data bytes with the isolated SPU module `aim_spu_module.self`
[OFW-4.93 lv1 a:0x593850] [OFW-4.93 lv1 a:0x56E5C8-0x56E5E4]. The
module reads data of the console that another request gives
[OFW-4.93 lv1 a:0x56E454-0x56E46C]. So no image read gives the data
bytes.

The class writes two fields of the reply:

- the status word, at `DISPATCHER_REPLY_STATUS_OFFSET`
  [OFW-4.93 lv1 a:0x58D67C-0x58D6A8];
- the 16 data bytes, at `DISPATCHER_REPLY_DATA_OFFSET`
  [OFW-4.93 lv1 a:0x56D234-0x56D248].

Each other byte of the reply is the request byte at the same offset.

The dispatcher manager keeps the tag and the reply size of the
request's header [OFW-4.93 lv1 a:0x5BDC08-0x5BDC18]
[OFW-4.93 lv1 a:0x5BC98C-0x5BC9A8]. When the service replies, it
writes a header to the port, and then the reply bytes that the header
counts [OFW-4.93 lv1 a:0x5BD4FC-0x5BD540]; see
`dispatcher_reply_header`.

The kernel reads these fields of the reply:

- The request id at +0. The base unmarshal refuses a reply with
  another id [OFW-4.93 lv2_kernel a:0x8000000000212d18-0x8000000000212d40].
- The status word at +16, which it keeps at +24 of the request object
  [OFW-4.93 lv2_kernel a:0x8000000000212d44-0x8000000000212d6c].
- The 16 data bytes at +40
  [OFW-4.93 lv2_kernel a:0x8000000000229ab0-0x8000000000229b38].

The kernel keeps the status and the data bytes in a cache object. The
capture `DISPATCHER_REPLY_CAPTURE` reads that object on the console;
see `GAME_OS_DISPATCHER_REPLIES`.

### The kernel's request for random bytes

In CellGov's boot, the kernel's second request to the dispatcher
manager is request 0x5003 of service 0x5000; see
`GAME_OS_RANDOM_DISPATCHER_REQUESTS`. The kernel uses its reply in
this way:

- The request object's constructor puts the request id at +8 and the
  service id at +16
  [OFW-4.93 lv2_kernel a:0x8000000000227fa4-0x8000000000227fdc].
- The class unmarshal copies the 24 reply bytes at +40 to the caller's
  buffer [OFW-4.93 lv2_kernel a:0x8000000000227fe0-0x80000000002280dc].
- The caller keeps the first 20 bytes as the seed of the kernel's
  random number generator
  [OFW-4.93 lv2_kernel a:0x8000000000228424-0x800000000022858c]
  [OFW-4.93 lv2_kernel a:0x8000000000228600-0x8000000000228718]
  [OFW-4.93 lv2_kernel a:0x8000000000228788-0x80000000002287d0].

The dispatcher manager's map gives service 0x5000 the socket address
0x23 [OFW-4.93 lv1 a:0x5D70D0-0x5D70DC]. A process of lv1 binds a
socket to that address [OFW-4.93 lv1 a:0x4EA9AC-0x4EA9C8]. Its class
for request 0x5003 [OFW-4.93 lv1 a:0x506C10-0x506C3C] answers in this
order:

1. The class keeps the address of the 24 bytes at +40 of the request
   buffer [OFW-4.93 lv1 a:0x5076A4-0x5076E8] [OFW-4.93 lv1 a:0x5279C4].
2. The handler stores its worker's return value as the status
   [OFW-4.93 lv1 a:0x508D60-0x508D84].
3. The worker writes the 24 bytes at that address
   [OFW-4.93 lv1 a:0x50B490-0x50B550].

The worker gets the bytes from one of two sources:

- A generator that it seeds from the time base, when process system
  call 0x100C1 gives 1 for the keys `ss`, `fw`, `dummy` and `rnd`
  [OFW-4.93 lv1 a:0x50BB98-0x50BC44] [OFW-4.93 lv1 a:0x529864-0x5298A0]
  [OFW-4.93 lv1 a:0x50B4BC-0x50B50C].
- Otherwise, the isolated SPU module `sb_iso_spu_module.self`
  [OFW-4.93 lv1 a:0x51D674-0x51D718] [OFW-4.93 lv1 a:0x51DAD0-0x51DAE8]
  [OFW-4.93 lv1 a:0x53F6F8-0x53F70C]. The function that takes the 24
  bytes from the module's result logs as `sbm_get_rnd`
  [OFW-4.93 lv1 a:0x51DA00-0x51DA84] [OFW-4.93 lv1 a:0x53F6A8-0x53F6F4].

So no image read gives the 24 bytes.

The status 0 is the path that gives the kernel its seed:

- The worker writes the 24 bytes only on a path where it returns 0
  [OFW-4.93 lv1 a:0x50B51C] [OFW-4.93 lv1 a:0x50B534-0x50B558].
- The kernel's request function returns the status word that it keeps
  [OFW-4.93 lv2_kernel a:0x80000000002283b8-0x80000000002283ec].
- The caller keeps the seed only when that status is 0
  [OFW-4.93 lv2_kernel a:0x8000000000228470-0x800000000022847c].

### The kernel's identity request

The kernel's request 0x19004 of service 0x19000 gets reply data that
no image read gives; see `GAME_OS_IDENTITY_DISPATCHER_REQUESTS`. The
kernel's marshal for the request writes the 8 bytes at +50 of the
object to +40 of the request bytes
[OFW-4.93 lv2_kernel a:0x8000000000229ecc-0x8000000000229f14].

`ss_server2` answers the request in this order:

1. Its class for request 0x19004 keeps an 8-byte field at +50
   [OFW-4.93 lv1 a:0x56CA14-0x56CA3C].
2. The server calls slot 9 of the class's table, which holds the
   handler [OFW-4.93 lv1 a:0x58DFC4] [OFW-4.93 lv1 a:0x5986E4]
   [OFW-4.93 lv1 a:0x59CCE8].
3. The handler gives the field to its worker and stores the worker's
   return value as the status [OFW-4.93 lv1 a:0x56D314-0x56D338].
4. The worker runs function 3 of the isolated SPU module that builds
   the data of request 0x19002 [OFW-4.93 lv1 a:0x56DC18-0x56DC2C]
   [OFW-4.93 lv1 a:0x56E1A4] [OFW-4.93 lv1 a:0x56E230-0x56E240]
   [OFW-4.93 lv1 a:0x56DAF0].

So no image read gives the data bytes. The module derives them from
data of the console.

## Logical SPE

The facts live in `cellgov_ps3_abi::lv1::spe`. The sections follow a
type 1 logical SPE from its construction, through its area, its shadow
page and its irq outlets, to the calls that set its transition
notifier, its run timeout and its interrupt masks.

### Constructing a logical SPE

The handler of `construct_logical_spe` (hypercall 57)
[OFW-4.93 lv1 a:0x31AB6C] gives r3 to r9 and the addresses of six stack
slots to the worker [OFW-4.93 lv1 a:0x31AC64]. It then copies the six
slots to the return slots of r4 to r9
[OFW-4.93 lv1 a:0x31AC68-0x31ACA4], and returns the worker's r3
[OFW-4.93 lv1 a:0x31AD08]. It copies the slots on each path, and a path
that fails leaves some slots unwritten. So r4 to r9 of a failed call
are stack values that no source gives. The handler writes no return
slot of r10, and the vector's entry zeroes the slots
[OFW-4.93 lv1 a:0x0C08]. So r10 returns 0.

The worker [OFW-4.93 lv1 a:0x31A34C] makes these checks in this order:

1. Each page shift in r3 to r7 is 12 to 61. The first that is not
   returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked with bit 0 to 4
   of the high word for r3 to r7
   [OFW-4.93 lv1 a:0x31A35C-0x31A3E0] [OFW-4.93 lv1 a:0x31A5D4-0x31A634].
2. Each page shift is 48 or less. A larger one returns
   `LV1_RESOURCE_SHORTAGE` (-2), zero-extended
   [OFW-4.93 lv1 a:0x31A3E4-0x31A45C].
3. r8 names a virtual address space in the partition's object table.
   A failed lookup returns `LV1_NO_ENTRY` (-6), marked with bit 5
   [OFW-4.93 lv1 a:0x31A460-0x31A598] [OFW-4.93 lv1 a:0x31A668-0x31A674].
4. It stores 0 in the slot of r7 [OFW-4.93 lv1 a:0x31A688-0x31A68C].
5. It selects a worker by the SPE type in r9: type 0
   [OFW-4.93 lv1 a:0x31A7EC], type 1 [OFW-4.93 lv1 a:0x31A72C] or
   type 2 [OFW-4.93 lv1 a:0x31A790]. Another type returns
   `LV1_ILLEGAL_PARAMETER_VALUE`, marked with bit 6
   [OFW-4.93 lv1 a:0x31A6A4-0x31A6B4].

Before the type 1 and type 2 workers, the worker returns a nonzero
status of the function [OFW-4.93 lv1 a:0x31D7D8]
[OFW-4.93 lv1 a:0x31A678-0x31A680] [OFW-4.93 lv1 a:0x31A6B8-0x31A6BC]
[OFW-4.93 lv1 a:0x31A730-0x31A734]. Type 1 also needs a flag of lv1's,
else it returns `LV1_CONDITION_NOT_SATISFIED` (-16), zero-extended
[OFW-4.93 lv1 a:0x31A6C0-0x31A6D0] [OFW-4.93 lv1 a:0x31A7F0-0x31A7F8].
The worker gives the type worker the partition, r8, the page shift of
r7 and five of the slots [OFW-4.93 lv1 a:0x31A6D4-0x31A72C].

### The type 1 worker

The type 1 worker [OFW-4.93 lv1 a:0x31588C] does these steps:

1. It returns `LV1_DENIED_BY_POLICY` (-4), zero-extended, when a flag
   of lv1's is 0
   [OFW-4.93 lv1 a:0x3158F8-0x315904] [OFW-4.93 lv1 a:0x315BF8-0x315C04].
2. It allocates a 592-byte SPE object [OFW-4.93 lv1 a:0x31590C-0x315924]
   and constructs it [OFW-4.93 lv1 a:0x315948]. The constructor calls
   the object constructor [OFW-4.93 lv1 a:0x2FA650]. So the object's
   id at +72 is the first free id of the partition's object table; see
   `lv1::event_port`.
3. It creates the SPE's area [OFW-4.93 lv1 a:0x3159EC]: a region of
   `LOGICAL_SPE_AREA_SIZE` bytes in 4 KB pages
   [OFW-4.93 lv1 a:0x2FED4C-0x2FED58].
4. It creates the shadow page [OFW-4.93 lv1 a:0x3159F8-0x3159FC]: a
   region of 2^p bytes in 2^p-byte pages, where p is the page shift of
   r7 [OFW-4.93 lv1 a:0x2FF554-0x2FF580].
5. It writes the slots and returns 0
   [OFW-4.93 lv1 a:0x315CB0-0x315CE4] [OFW-4.93 lv1 a:0x315D3C]:
   - r4 is the area's address plus `LOGICAL_SPE_PRIV2_OFFSET`;
   - r5 is the area's address plus `LOGICAL_SPE_PROBLEM_OFFSET`;
   - r6 is the area's address: the local store;
   - r8 is the shadow page's address;
   - r9 is the object's id.

A failed allocation in the type 1 worker returns
`LV1_RESOURCE_SHORTAGE`, zero-extended [OFW-4.93 lv1 a:0x315B28-0x315B30].
A nonzero status of the constructor [OFW-4.93 lv1 a:0x3159D4-0x3159E4]
and a region that lv1 cannot create [OFW-4.93 lv1 a:0x315A00-0x315A14]
[OFW-4.93 lv1 a:0x315C14-0x315C1C] return it too.

### The regions

Both regions take their address from the region function
[OFW-4.93 lv1 a:0x2CEC78], with the partition's region context
[OFW-4.93 lv1 a:0x2FED24-0x2FED48] [OFW-4.93 lv1 a:0x2FF540-0x2FF56C].
`allocate_memory` uses the same function and context
[OFW-4.93 lv1 a:0x2DEA68-0x2DEA94]. The function takes the size
exponent of the region [OFW-4.93 lv1 a:0x2CED34-0x2CED48]. It
increments the context's counter [OFW-4.93 lv1 a:0x2CED84-0x2CED94],
and encodes the old value in the address
[OFW-4.93 lv1 a:0x2CED98-0x2CEDA8]; see `lv1::memory`. So the area
takes a value of the partition's region counter, and the shadow page
takes the next value.

### The shadow page

lv1 backs the shadow page with a page of its own memory. The page
allocator fills the page with zeros [OFW-4.93 lv1 a:0x2FF5C4]
[OFW-4.93 lv1 a:0x2C14F8-0x2C1508]. The region keeps the page's real
address at +168 [OFW-4.93 lv1 a:0x2FF5D8]. The type 1 worker then
writes 0 to the fields of Linux `struct spe_shadow`
[OFW-4.93 lv1 a:0x315A1C-0x315A2C] [OFW-4.93 lv1 a:0x2FC250-0x2FC2EC].
So after the call each byte of the shadow page is 0.

`write_htab_entry` (hypercall 1) finds the region that holds an
entry's page in the partition's regions [OFW-4.93 lv1 a:0x2D2510]
[OFW-4.93 lv1 a:0x2D2614-0x2D262C]. It then asks the region's class
for the real address [OFW-4.93 lv1 a:0x2D3084-0x2D30C4]. The class of
the shadow page [OFW-4.93 lv1 a:0x2FF5B4-0x2FF5B8] translates at
[OFW-4.93 lv1 a:0x2FF754], with these steps in this order:

1. It checks a byte of the region that the region constructor sets
   to 0 [OFW-4.93 lv1 a:0x2FF384-0x2FF38C] [OFW-4.93 lv1 a:0x2CF4FC].
   A live shadow page passes.
2. A page larger than the shadow page fails; see
   `WRITE_HTAB_SHADOW_PAGE_SIZE_R3`.
3. An entry that is not read only fails; see
   `WRITE_HTAB_SHADOW_WRITABLE_R3`.
4. The real address is the backing page's address plus the page's
   offset in the region [OFW-4.93 lv1 a:0x2FF7E4-0x2FF7F0].

The writer searches for a duplicate entry before it translates
[OFW-4.93 lv1 a:0x2D2F20-0x2D3038].

### The irq outlets

The type 1 worker gives the SPE one irq outlet for each interrupt
class from 0 to 2 [OFW-4.93 lv1 a:0x315A38-0x315AE8]:

- It allocates a 160-byte object and constructs it as an outlet
  [OFW-4.93 lv1 a:0x315A84-0x315AAC].
- The outlet constructor [OFW-4.93 lv1 a:0x2FF934] calls the event
  receive port constructor [OFW-4.93 lv1 a:0x2FF954]. So the outlet
  takes the first free id of the partition's object table; see
  `lv1::event_port`.
- It stores the outlets of classes 0, 1 and 2 at +120, +128 and +136
  of the SPE [OFW-4.93 lv1 a:0x315AEC-0x315B10].

### Getting an irq outlet

The handler of `get_spe_irq_outlet` (hypercall 78)
[OFW-4.93 lv1 a:0x2FC968] gives r3, r4 and the address of a stack slot
to the worker [OFW-4.93 lv1 a:0x2FC9F0-0x2FC9FC]. It then copies the
slot to the return slot of r4 and returns the worker's r3
[OFW-4.93 lv1 a:0x2FCA00-0x2FCA14] [OFW-4.93 lv1 a:0x2FCA78]. It copies
the slot on each path, and a failing worker leaves the slot unwritten.
So r4 of a failed call is a stack value that no source gives. The
handler writes no return slot of r5 to r10, and the vector's entry
zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0.

The worker [OFW-4.93 lv1 a:0x2FC738] does these steps in this order:

1. A class above 2 returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked
   with bit 1 of the high word for r4
   [OFW-4.93 lv1 a:0x2FC738] [OFW-4.93 lv1 a:0x2FC764]
   [OFW-4.93 lv1 a:0x2FC8F8-0x2FC908].
2. It looks up r3 in the partition's object table, and checks that
   the object's class word at +48 is the SPE class in the TOC slot at
   r2 - 4704 [OFW-4.93 lv1 a:0x2FC768-0x2FC88C]. A failed check
   returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
   [OFW-4.93 lv1 a:0x2FC890-0x2FC89C].
3. It calls the method in the TOC slot at r2 - 4696 on the SPE
   [OFW-4.93 lv1 a:0x2FC90C-0x2FC964]: the function descriptor
   [OFW-4.93 lv1 a:0x34AAF0] of the method [OFW-4.93 lv1 a:0x2FA588].
   The call function [OFW-4.93 lv1 a:0x2F999C] takes the SPE's lock at
   +216 [OFW-4.93 lv1 a:0x2F99A4-0x2F9A94]. It calls the method when
   bits 36 to 38 (mask 0x0000007000000000) of the doubleword at +184
   are 0 [OFW-4.93 lv1 a:0x2F9A98-0x2F9AE8], and returns the method's
   r3. The type 1 worker sets that doubleword to 0x0002000000000000
   [OFW-4.93 lv1 a:0x315928-0x315948] [OFW-4.93 lv1 a:0x2FA6AC], so a
   new SPE passes.
4. The method stores the id at +72 of the class's outlet in the slot,
   and returns 0 [OFW-4.93 lv1 a:0x2FA588-0x2FA5A0].

### Setting the transition notifier

The handler of `set_spe_transition_notifier` (hypercall 64)
[OFW-4.93 lv1 a:0x2FCF68] gives r3, r4 and r5 to the worker
[OFW-4.93 lv1 a:0x2FCFF8-0x2FD004]. It returns the worker's r3
[OFW-4.93 lv1 a:0x2FD070], and writes no return slot. The vector's
entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0
on each path.

The worker [OFW-4.93 lv1 a:0x2FCA94] does these steps in this order:

1. It looks up r3 in the partition's object table, and checks that
   the object's class word at +48 is the SPE class in the TOC slot at
   r2 - 4704 [OFW-4.93 lv1 a:0x2FCAC0-0x2FCBE0]. A failed check
   returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
   [OFW-4.93 lv1 a:0x2FCBE4-0x2FCBF0].
2. When r5 is 0, it calls the method with no port and a mask of 0
   [OFW-4.93 lv1 a:0x2FCC58-0x2FCC5C] [OFW-4.93 lv1 a:0x2FCF38-0x2FCF64].
   It does not read r4 on this path.
3. It looks up r5 in the object table, and checks these fields of the
   object [OFW-4.93 lv1 a:0x2FCC60-0x2FCE98]:

   - the class word at +48 is the port class in the TOC slot at
     r2 - 4688;
   - the word at +136 is the word in the TOC slot at r2 - 4680;
   - the byte at +148 is not 0.

   A failed check returns `LV1_NO_ENTRY`, marked with bit 2 for r5
   [OFW-4.93 lv1 a:0x2FCD78-0x2FCD88].
4. It calls the method with the port and with r4 as the mask
   [OFW-4.93 lv1 a:0x2FCE9C-0x2FCEC4], and returns the method's r3.

Both calls go through the call function of `get_spe_irq_outlet`, with
the method in the TOC slot at r2 - 4672. That slot holds the function
descriptor [OFW-4.93 lv1 a:0x34AAD8] of the method
[OFW-4.93 lv1 a:0x2FA4E8]. A new SPE passes the check of the call
function on its doubleword at +184.

### The objects that the notifier accepts

A port that `construct_event_receive_port` constructs passes each
check of step 3; see `lv1::event_port`:

- The port constructor gives the object constructor the class in the
  TOC slot at r2 - 8032 [OFW-4.93 lv1 a:0x2DF61C]
  [OFW-4.93 lv1 a:0x2CC6A8]. That slot and the slot at r2 - 4688
  hold the same class [OFW-4.93 lv1 a:0x3580D8]
  [OFW-4.93 lv1 a:0x358DE8].
- The port constructor stores its r6 at +136
  [OFW-4.93 lv1 a:0x2DF664]. The function of hypercall 18 gives it the
  word in the TOC slot at r2 - 7896 [OFW-4.93 lv1 a:0x2E2538]. That
  slot and the slot at r2 - 4680 hold the same word
  [OFW-4.93 lv1 a:0x358160] [OFW-4.93 lv1 a:0x358DF0].
- The byte at +148 is 1 when the object constructor succeeds.

An irq outlet fails step 3. Its constructor gives the port
constructor the word in the TOC slot at r2 - 4472
[OFW-4.93 lv1 a:0x2FF94C], which holds a different word
[OFW-4.93 lv1 a:0x358EC0].

An irq outlet passes each check of `connect_irq_plug_ext` that a port
passes; see `lv1::irq_plug`:

- The outlet constructor gives the port constructor its own r3 to r5
  [OFW-4.93 lv1 a:0x2FF934-0x2FF954]. So the outlet's class word at
  +48 is the port class, from the TOC slot at r2 - 8032.
- The handler compares the class word with the class in the TOC slot
  at r2 - 7072 [OFW-4.93 lv1 a:0x2EA18C-0x2EA198]. lv1 runs with the
  TOC 0x35A038; see `lv1::dispatch`. That slot and the slot at
  r2 - 8032 hold the same class
  [OFW-4.93 lv1 a:0x358498] [OFW-4.93 lv1 a:0x3580D8].
- The port constructor sets +148 to 1 when the object constructor
  writes no status [OFW-4.93 lv1 a:0x2DF668-0x2DF694]. The type 1
  worker gives it a status slot of 0, and continues only when the slot
  is still 0
  [OFW-4.93 lv1 a:0x315A7C-0x315A8C] [OFW-4.93 lv1 a:0x315ABC-0x315ACC].
- After the port constructor, the outlet constructor writes only +0
  and +152 of the outlet [OFW-4.93 lv1 a:0x2FF958-0x2FF968]. The
  function that it calls next links the outlet to the 152-byte object
  through the list at +104 of the outlet
  [OFW-4.93 lv1 a:0x2CB224] [OFW-4.93 lv1 a:0x2CB274-0x2CB2A4]. So the
  connection at +128, the pending byte at +149 and the list at +96 keep
  the values that the port constructor gave them.
- The type 1 worker puts each outlet, and then the SPE, in the
  partition's object table [OFW-4.93 lv1 a:0x315C90-0x315CAC]. The
  function stores an object in the slot that its id at +72 names
  [OFW-4.93 lv1 a:0x2CB6B8-0x2CB758].

### The notifier method

The method [OFW-4.93 lv1 a:0x2FA4E8] does these steps in this order:

1. A mask bit that is not set in the doubleword at +160 of the SPE
   returns `LV1_ILLEGAL_PARAMETER_VALUE` (-17), marked with bit 1 for
   r4 [OFW-4.93 lv1 a:0x2FA4FC-0x2FA52C].
2. It adds a reference to the new port when the port is not 0, and
   stores the port at +144 of the SPE. It then removes a reference
   from the port that +144 held when that port is not 0
   [OFW-4.93 lv1 a:0x2FA530-0x2FA548] [OFW-4.93 lv1 a:0x2FA570-0x2FA584].
   The reference count is the word at +8 of the port
   [OFW-4.93 lv1 a:0x2BED58-0x2BED6C] [OFW-4.93 lv1 a:0x2BEE78-0x2BEE8C].
3. It stores the mask at +152 of the SPE, and returns 0
   [OFW-4.93 lv1 a:0x2FA54C-0x2FA550].

The SPE constructor sets +144, +152 and +160 to 0
[OFW-4.93 lv1 a:0x2FA680-0x2FA688]. The type 1 worker then sets +160
to `LOGICAL_SPE_TYPE_1_TRANSITION_MASK`. A call with r5 = 0 passes
step 1 of the method with the mask 0. It then stores 0 at +144 and
+152, the values that the constructor set, and removes a reference
from the port of an earlier call [OFW-4.93 lv1 a:0x2FA530-0x2FA550].
So it clears the notifier of an earlier call.

### Setting the run timeout

The handler of `logical_spu_set_run_timeout` (hypercall 138)
[OFW-4.93 lv1 a:0x31BD54] gives r3 and r4 to the worker
[OFW-4.93 lv1 a:0x31BDDC-0x31BDE4]. It returns the worker's r3
[OFW-4.93 lv1 a:0x31BE50], and writes no return slot. The vector's
entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0
on each path.

The worker [OFW-4.93 lv1 a:0x31BAF0] does these steps in this order:

1. It calls the function [OFW-4.93 lv1 a:0x31D7D8] with the calling
   partition and 1, and returns a nonzero status
   [OFW-4.93 lv1 a:0x31BB10-0x31BB2C].
2. It returns `LV1_CONDITION_NOT_SATISFIED` (-16), zero-extended, when
   a flag of lv1's is 0
   [OFW-4.93 lv1 a:0x31BB30-0x31BB40] [OFW-4.93 lv1 a:0x31BCC4-0x31BCD0].
3. It looks up r3 in the partition's object table, and checks these
   fields of the object [OFW-4.93 lv1 a:0x31BB44-0x31BD0C]:

   - the class word at +48 is the SPE class in the TOC slot at
     r2 - 128;
   - the doubleword at +192 is 0x0002000000000000.

   A failed check returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
   [OFW-4.93 lv1 a:0x31BC60-0x31BC6C].
4. It gives r4 to the method in the TOC slot at r2 - 80, through the
   call function [OFW-4.93 lv1 a:0x31A028], and returns the method's r3
   [OFW-4.93 lv1 a:0x31BD18-0x31BD50]. That slot holds the function
   descriptor [OFW-4.93 lv1 a:0x351348] of the method
   [OFW-4.93 lv1 a:0x315688].

### The run timeout of a type 1 SPE

On the type 1 path, the worker of `construct_logical_spe`
[OFW-4.93 lv1 a:0x31A34C] makes the same call of
step 1 [OFW-4.93 lv1 a:0x31A678-0x31A680], and tests the same flag
as step 2 [OFW-4.93 lv1 a:0x31A6C0-0x31A6D0]. A type 1 SPE passes the
checks of step 3:

- The SPE constructor gives the object constructor the class in the
  TOC slot at r2 - 5008 [OFW-4.93 lv1 a:0x2FA634]
  [OFW-4.93 lv1 a:0x2FA650]. The object constructor stores it at +48
  [OFW-4.93 lv1 a:0x2CC6A8]. The slots at r2 - 5008 and r2 - 128
  hold the same class [OFW-4.93 lv1 a:0x358CA8]
  [OFW-4.93 lv1 a:0x359FB8]. The descriptors of this handler and of
  the `get_spe_irq_outlet` method beside the constructor give the same
  TOC, 0x35A038 [OFW-4.93 lv1 a:0x351C6C] [OFW-4.93 lv1 a:0x34AAFC].
- The type 1 worker gives the SPE constructor 0x0002000000000000 in r7
  and r8 [OFW-4.93 lv1 a:0x315928-0x315948]. The constructor stores r8
  at +184 and r7 at +192
  [OFW-4.93 lv1 a:0x2FA638] [OFW-4.93 lv1 a:0x2FA64C]
  [OFW-4.93 lv1 a:0x2FA6AC-0x2FA6B4].

The call function takes the SPE's lock at +216
[OFW-4.93 lv1 a:0x31A030-0x31A118]. It then reads the doubleword at
+184:

- Bits 0x0000007000000000 are 0: it calls the method, and returns the
  method's r3 [OFW-4.93 lv1 a:0x31A11C-0x31A18C].
- Bit 0x0000001000000000 is set: it returns `LV1_NO_ENTRY` (-6),
  marked with bit 0 for r3
  [OFW-4.93 lv1 a:0x31A1F0-0x31A200] [OFW-4.93 lv1 a:0x31A338-0x31A344].
- Otherwise it waits until bit 0x0000002000000000 is clear, and reads
  +184 again [OFW-4.93 lv1 a:0x31A240-0x31A334].

The method [OFW-4.93 lv1 a:0x315688] does these steps in this order:

1. A doubleword at +184 that is not 0x0002000000000000 returns
   `LV1_WRONG_STATE` (-11), marked with bit 0 for r3
   [OFW-4.93 lv1 a:0x315688-0x3156AC].
2. It stores r4 at +584 of the SPE, and returns 0
   [OFW-4.93 lv1 a:0x3156B0-0x3156B8].

The type 1 worker sets +584 to 0
[OFW-4.93 lv1 a:0x315908] [OFW-4.93 lv1 a:0x315958]. The start step
of the SPE reads +584 [OFW-4.93 lv1 a:0x3165B8-0x3165C0]. When it is
not 0, the step adds it to the time base, and arms a timer for that
deadline [OFW-4.93 lv1 a:0x3165C4-0x316604]. So r4 is the SPE's run
time limit in time base ticks, and 0 is no limit.

### Setting an interrupt mask

The handler of `set_spe_interrupt_mask` (hypercall 61)
[OFW-4.93 lv1 a:0x2FE570] gives r3, r4 and r5 to the worker
[OFW-4.93 lv1 a:0x2FE600-0x2FE60C]. It returns the worker's r3
[OFW-4.93 lv1 a:0x2FE614] [OFW-4.93 lv1 a:0x2FE678], and writes no
return slot. The vector's entry zeroes the slots
[OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0 on each path.

The worker [OFW-4.93 lv1 a:0x2FE340] does these steps in this order:

1. A class above 2 returns `SET_SPE_INTERRUPT_MASK_CLASS_R3`. The
   worker does not read r3 or r5 on this path.
2. It looks up r3 in the partition's object table, and checks that
   the object's class word at +48 is the SPE class in the TOC slot at
   r2 - 4704 [OFW-4.93 lv1 a:0x2FE370-0x2FE494]. A failed check
   returns `LV1_NO_ENTRY` (-6), marked with bit 0 for r3
   [OFW-4.93 lv1 a:0x2FE498-0x2FE4A4].
3. It gives r4 and r5 to the method in the TOC slot at r2 - 4616,
   through the call function of `get_spe_irq_outlet`, and returns the
   method's r3 [OFW-4.93 lv1 a:0x2FE530-0x2FE56C]. That slot holds the
   function descriptor [OFW-4.93 lv1 a:0x34AA78] of the method
   [OFW-4.93 lv1 a:0x2FA23C]. The call function gives the method the
   SPE, r4 and r5 [OFW-4.93 lv1 a:0x2F9AAC-0x2F9AE8]. A new SPE passes
   its check on the doubleword at +184.

The method [OFW-4.93 lv1 a:0x2FA23C] does these steps in this order:

1. It takes the outlet of the class from +120 + 8 * class of the SPE
   [OFW-4.93 lv1 a:0x2FA23C-0x2FA254].
2. It ANDs r5 with the entry of the class in
   `SPE_INTERRUPT_CLASS_MASKS` [OFW-4.93 lv1 a:0x2FA24C-0x2FA25C].
3. It gives the result to the method at +48 of the class of the
   outlet's interrupt object at +152 [OFW-4.93 lv1 a:0x2FA260-0x2FA27C].
4. A nonzero return gives `LV1_WRONG_STATE` (-11), marked with bit 0
   for r3. Otherwise it returns 0 [OFW-4.93 lv1 a:0x2FA280-0x2FA2AC].

### The interrupt object of an outlet

The type 1 worker makes a 152-byte interrupt object before each outlet
[OFW-4.93 lv1 a:0x315A40-0x315A70]. It gives the object to the outlet
constructor [OFW-4.93 lv1 a:0x315AA4-0x315AAC], which stores it at
+152 of the outlet [OFW-4.93 lv1 a:0x2FF968]. The object's constructor
[OFW-4.93 lv1 a:0x3192C0] does these steps:

- Its base constructor [OFW-4.93 lv1 a:0x2FC370] stores the address of
  the SPE's record at +224 at +88, the class at +96 and 0 at +112
  [OFW-4.93 lv1 a:0x2FC3A8-0x2FC3B8]. The type 1 worker keeps that
  address at +112 of the SPE [OFW-4.93 lv1 a:0x315A18]
  [OFW-4.93 lv1 a:0x315A34], and gives it as r4
  [OFW-4.93 lv1 a:0x315A5C].
- It stores the class in the TOC slot at r2 - 344 at +0
  [OFW-4.93 lv1 a:0x3192F0-0x3192F8]. That slot holds the class
  [OFW-4.93 lv1 a:0x359EE0]. The class's entry at +48
  [OFW-4.93 lv1 a:0x363D28] holds the function descriptor
  [OFW-4.93 lv1 a:0x3519D8] of the mask method
  [OFW-4.93 lv1 a:0x3198F4].

The mask method takes the lock at +48 of the object
[OFW-4.93 lv1 a:0x3198FC-0x319948]. It then reads the doubleword at +0
of the SPE's record at +224 [OFW-4.93 lv1 a:0x31994C-0x319958]:

- 0: it stores the mask at +112 of the object, and returns 0
  [OFW-4.93 lv1 a:0x3199F4-0x3199F8].
- Not 0: the doubleword is a physical SPU's record. A check of that
  record that fails returns 1 [OFW-4.93 lv1 a:0x31995C-0x319974].
  Otherwise the method stores the mask at +112 of the object and at
  +0x100 + 8 * class of the address at +16 of the record, and returns
  0 [OFW-4.93 lv1 a:0x319978-0x319990]. +0x100 + 8 * class is the
  offset of the class's interrupt mask register in an SPE's privilege
  1 area: 0x100, 0x108 and 0x110 for classes 0 to 2
  [CBEA p:270] [CBEA p:271] [CBEA p:272].

The type 1 worker gives the SPE's record at +224 to the function
[OFW-4.93 lv1 a:0x2FC250] [OFW-4.93 lv1 a:0x315A18-0x315A2C], through
the branch at [OFW-4.93 lv1 a:0x319BD8] that its call targets, which
stores 0 at +0 of the record [OFW-4.93 lv1 a:0x2FC284]. So a new SPE
takes the first path, and the mask of each class starts at 0.
