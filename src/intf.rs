//intf
//

use crate::cdc_ncm::CdcNcmClass;

use core::mem::size_of;
use usb_device::bus::UsbBus;
use usb_device::class_prelude::*;
use core::array::TryFromSliceError;
extern crate alloc;



#[derive(Clone, Copy)]
struct CdcSpeedChangeBody {
    bitrate_dl: u32,
    bitrate_ul: u32,
}

#[derive(Clone, Copy)]
struct NotifyHeader {
    requestype: u8,
    notificationtype: u8,
    value: u16,
    index: u16,
    length: u16,
}


#[repr(C)]
#[derive(Clone, Copy)]
struct CdcSpeedChangeMsg {
    header: NotifyHeader,
    body: CdcSpeedChangeBody,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CdcConnectionNotifyMsg {
    header: NotifyHeader,
}

impl Default for CdcSpeedChangeMsg {
    fn default() -> Self {
        CdcSpeedChangeMsg {
            header: NotifyHeader {
                requestype: 0xA1,
                notificationtype: 0x2A,
                value: 1,
                index: 1,
                length: size_of::<CdcSpeedChangeBody>() as u16,
            },
            body: CdcSpeedChangeBody {
                bitrate_dl: 10 * 1000000,
                bitrate_ul: 10 * 1000000,
            },
        }
    }
}

impl TryInto<[u8; size_of::<CdcSpeedChangeMsg>()]> for CdcSpeedChangeMsg {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<[u8; size_of::<CdcSpeedChangeMsg>()], Self::Error> {
        const ARRLEN: usize = size_of::<CdcSpeedChangeMsg>();
        let mut arr: [u8; ARRLEN] = [0; ARRLEN];
        arr[0] = self.header.requestype;
        arr[1] = self.header.notificationtype;
        arr[2..4].copy_from_slice(&self.header.value.to_le_bytes());
        arr[4..6].copy_from_slice(&self.header.index.to_le_bytes());
        arr[6..8].copy_from_slice(&self.header.length.to_le_bytes());
        arr[8..12].copy_from_slice(&self.body.bitrate_dl.to_le_bytes());
        arr[12..16].copy_from_slice(&self.body.bitrate_ul.to_le_bytes());
        Ok(arr)
    }
}

impl Default for CdcConnectionNotifyMsg {
    fn default() -> Self {
        CdcConnectionNotifyMsg {
            header: NotifyHeader {
                requestype: 0xA1,
                notificationtype: 0x00,
                value: 1,
                index: 1,
                length: 0,
            },
        }
    }
}

impl TryInto<[u8; size_of::<CdcConnectionNotifyMsg>()]> for CdcConnectionNotifyMsg {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<[u8; size_of::<CdcConnectionNotifyMsg>()], Self::Error> {
        const ARRLEN: usize = size_of::<CdcConnectionNotifyMsg>();
        let mut arr: [u8; ARRLEN] = [0; ARRLEN];
        arr[0] = self.header.requestype;
        arr[1] = self.header.notificationtype;
        arr[2..4].copy_from_slice(&self.header.value.to_le_bytes());
        arr[4..6].copy_from_slice(&self.header.index.to_le_bytes());
        arr[6..8].copy_from_slice(&self.header.length.to_le_bytes());
        Ok(arr)
    }
}


pub struct UsbIp<'a, B:UsbBus>
{
    pub inner: CdcNcmClass<'a, B>,
}

impl<B: UsbBus> UsbIp<'_, B>
{
    /// Creates a new USB serial port with the provided UsbBus and 128 byte read/write buffers.
    pub fn new(alloc: &'_ UsbBusAllocator<B>) -> UsbIp<'_, B> {
        UsbIp {
            inner: CdcNcmClass::new(alloc),
        }
    }

    pub fn send_speed_notificaiton(&mut self) -> usb_device::Result<usize>  {
        let speedmsg: [u8; size_of::<CdcSpeedChangeMsg>()] =
            CdcSpeedChangeMsg::default().try_into().unwrap();
        self.inner.send_notification(speedmsg.as_slice())
    }
    pub fn send_connection_notificaiton(&mut self) -> usb_device::Result<usize>  {
        //update internal state as connected
        // self.ip_in.borrow_mut().set_connection_state(true);
        let conmsg: [u8; size_of::<CdcConnectionNotifyMsg>()] =
            CdcConnectionNotifyMsg::default().try_into().unwrap();
        self.inner.send_notification(conmsg.as_slice())
    }

}

