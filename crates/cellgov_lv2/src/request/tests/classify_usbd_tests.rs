//! Register positions of the `sys_usbd` family and
//! `sys_memory_allocate_from_container`.

use super::*;

#[test]
fn classify_usbd_family_reads_each_register_position() {
    assert_eq!(
        classify(syscall::USBD_INITIALIZE, &[0x2000, 0x77, 0, 0, 0, 0, 0, 0]),
        Lv2Request::UsbdInitialize { handle_ptr: 0x2000 }
    );
    assert_eq!(
        classify(syscall::USBD_FINALIZE, &[0x115b, 0x77, 0, 0, 0, 0, 0, 0]),
        Lv2Request::UsbdFinalize { handle: 0x115b }
    );
    assert_eq!(
        classify(
            syscall::USBD_GET_DEVICE_LIST,
            &[0x115b, 0x6000, 4, 0x77, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdGetDeviceList {
            handle: 0x115b,
            list_ptr: 0x6000,
            max_devices: 4,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_GET_DESCRIPTOR_SIZE,
            &[0x115b, 0x1, 0x77, 0, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdGetDescriptorSize {
            handle: 0x115b,
            device: 1,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_GET_DESCRIPTOR,
            &[0x115b, 0x1, 0x6000, 0x40, 0x77, 0, 0, 0]
        ),
        Lv2Request::UsbdGetDescriptor {
            handle: 0x115b,
            device: 1,
            desc_ptr: 0x6000,
            desc_size: 0x40,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_REGISTER_LDD,
            &[0x115b, 0x5000, 8, 0x77, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdRegisterLdd {
            handle: 0x115b,
            product_ptr: 0x5000,
            product_len: 8,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_UNREGISTER_LDD,
            &[0x115b, 0x5000, 8, 0x77, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdUnregisterLdd {
            handle: 0x115b,
            product_ptr: 0x5000,
            product_len: 8,
        }
    );
    // endpoint is the sixth argument; the unknowns between are not
    // decoded.
    assert_eq!(
        classify(
            syscall::USBD_OPEN_PIPE,
            &[0x115b, 0x1, 0x77, 0x77, 0x77, 0x81, 0x77, 0]
        ),
        Lv2Request::UsbdOpenPipe {
            handle: 0x115b,
            device: 1,
            endpoint: 0x81,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_OPEN_DEFAULT_PIPE,
            &[0x115b, 0x1, 0x77, 0, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdOpenDefaultPipe {
            handle: 0x115b,
            device: 1,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_CLOSE_PIPE,
            &[0x115b, 0x7, 0x77, 0, 0, 0, 0, 0]
        ),
        Lv2Request::UsbdClosePipe {
            handle: 0x115b,
            pipe: 7,
        }
    );
    assert_eq!(
        classify(
            syscall::USBD_RECEIVE_EVENT,
            &[0x115b, 0x3000, 0x3008, 0x3010, 0x77, 0, 0, 0]
        ),
        Lv2Request::UsbdReceiveEvent {
            handle: 0x115b,
            arg1_ptr: 0x3000,
            arg2_ptr: 0x3008,
            arg3_ptr: 0x3010,
        }
    );
    assert_eq!(
        classify(syscall::USBD_DETECT_EVENT, &[0x77, 0x77, 0, 0, 0, 0, 0, 0]),
        Lv2Request::UsbdDetectEvent
    );
}

#[test]
fn a_usbd_event_wait_carries_no_timeout() {
    let req = classify(
        syscall::USBD_RECEIVE_EVENT,
        &[0x115b, 0x3000, 0x3008, 0x3010, 0, 0, 0, 0],
    );
    assert!(matches!(req, Lv2Request::UsbdReceiveEvent { .. }));
    assert_eq!(req.wait_timeout_usec(), None);
}

#[test]
fn classify_memory_allocate_from_container_keeps_size_and_flags_wide() {
    assert_eq!(
        classify(
            syscall::MEMORY_ALLOCATE_FROM_CONTAINER,
            &[0x1_0000_0000, 0x4000_0002, 0x200, 0x2000, 0x77, 0, 0, 0]
        ),
        Lv2Request::MemoryAllocateFromContainer {
            size: 0x1_0000_0000,
            cid: 0x4000_0002,
            flags: 0x200,
            alloc_addr_ptr: 0x2000,
        }
    );
}
