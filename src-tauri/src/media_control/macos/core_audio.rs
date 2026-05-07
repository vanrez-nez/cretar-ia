use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc;

pub const OK: i32 = 0;
pub const OBJECT_SYSTEM: u32 = 1;
pub const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
pub const SCOPE_OUTPUT: u32 = u32::from_be_bytes(*b"outp");
pub const ELEMENT_MAIN: u32 = 0;
pub const HARDWARE_DEFAULT_OUTPUT_DEVICE: u32 = u32::from_be_bytes(*b"dOut");
pub const DEVICE_NOMINAL_SAMPLE_RATE: u32 = u32::from_be_bytes(*b"nsrt");
pub const DEVICE_STREAM_CONFIGURATION: u32 = u32::from_be_bytes(*b"slay");

type ListenerProc = Option<
    unsafe extern "C" fn(
        object_id: u32,
        number_addresses: u32,
        addresses: *const PropertyAddress,
        client_data: *mut c_void,
    ) -> i32,
>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

impl PropertyAddress {
    pub fn new(selector: u32, scope: u32) -> Self {
        Self {
            selector,
            scope,
            element: ELEMENT_MAIN,
        }
    }

    pub fn selector(&self) -> u32 {
        self.selector
    }
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        object_id: u32,
        address: *const PropertyAddress,
        qualifier_data_size: u32,
        qualifier_data: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectGetPropertyDataSize(
        object_id: u32,
        address: *const PropertyAddress,
        qualifier_data_size: u32,
        qualifier_data: *const c_void,
        data_size: *mut u32,
    ) -> i32;
    fn AudioObjectAddPropertyListener(
        object_id: u32,
        address: *const PropertyAddress,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> i32;
    fn AudioObjectRemovePropertyListener(
        object_id: u32,
        address: *const PropertyAddress,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> i32;
}

pub fn read_u32(object_id: u32, address: PropertyAddress) -> Option<u32> {
    let mut value = 0u32;
    let mut data_size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            &address,
            0,
            ptr::null(),
            &mut data_size,
            &mut value as *mut u32 as *mut c_void,
        )
    };
    (status == OK).then_some(value)
}

pub fn read_f64(object_id: u32, address: PropertyAddress) -> Option<f64> {
    let mut value = 0f64;
    let mut data_size = std::mem::size_of::<f64>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object_id,
            &address,
            0,
            ptr::null(),
            &mut data_size,
            &mut value as *mut f64 as *mut c_void,
        )
    };
    (status == OK).then_some(value)
}

pub fn data_size(object_id: u32, address: PropertyAddress) -> Option<u32> {
    let mut data_size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(object_id, &address, 0, ptr::null(), &mut data_size)
    };
    (status == OK).then_some(data_size)
}

pub struct ListenerGuard<'a> {
    registrations: Vec<(u32, PropertyAddress)>,
    tx: &'a mpsc::Sender<()>,
}

impl<'a> ListenerGuard<'a> {
    pub fn new(tx: &'a mpsc::Sender<()>) -> Self {
        Self {
            registrations: Vec::new(),
            tx,
        }
    }

    pub fn add(&mut self, object_id: u32, address: PropertyAddress) {
        let status = unsafe {
            AudioObjectAddPropertyListener(
                object_id,
                &address,
                Some(route_listener),
                self.tx as *const _ as *mut c_void,
            )
        };
        if status == OK {
            self.registrations.push((object_id, address));
        } else {
            log::debug!(
                "media route: failed to register listener object_id={} selector={} status={}",
                object_id,
                address.selector(),
                status
            );
        }
    }
}

impl Drop for ListenerGuard<'_> {
    fn drop(&mut self) {
        for (object_id, address) in self.registrations.drain(..) {
            let _ = unsafe {
                AudioObjectRemovePropertyListener(
                    object_id,
                    &address,
                    Some(route_listener),
                    self.tx as *const _ as *mut c_void,
                )
            };
        }
    }
}

unsafe extern "C" fn route_listener(
    _object_id: u32,
    _number_addresses: u32,
    _addresses: *const PropertyAddress,
    client_data: *mut c_void,
) -> i32 {
    if !client_data.is_null() {
        let tx = &*(client_data as *const mpsc::Sender<()>);
        let _ = tx.send(());
    }
    OK
}
