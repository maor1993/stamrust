use crate::cdc_ncm::{
    CdcNcmClass,NDP16_SIGNATURE, NTH16_SIGNATURE,
};
use alloc::vec::Vec;
use core::array::TryFromSliceError;
use core::mem::size_of;
use defmt::debug;
use usb_device::bus::UsbBus;
use usb_device::class_prelude::*;
extern crate alloc;
// const PAGE_SIZE: usize = 2048;

// pub type NCMResult<T> = core::result::Result<T, NCMError>;


#[repr(C)]
#[derive(Debug, defmt::Format, Clone)]
pub struct NCMTransferHeader {
    pub signature: u32,
    pub headerlen: u16,
    pub sequence: u16,
    pub blocklen: u16,
    pub ndpindex: u16,
}

impl Default for NCMTransferHeader {
    fn default() -> Self {
        NCMTransferHeader {
            signature: u32::from_le_bytes(NTH16_SIGNATURE.try_into().unwrap()),
            headerlen: 0x000c,
            sequence: 0,
            blocklen: 0,
            ndpindex: 0x0010,
        }
    }
}

#[repr(C)]
#[derive(Debug, defmt::Format, Clone, Default)]
pub struct NCMDatagram16 {
    pub index: u16,
    pub length: u16,
}

#[repr(C)]
#[derive(Debug, defmt::Format, Clone)]
pub struct NCMDatagramPointerTable {
    pub signature: u32,
    pub length: u16,
    pub nextndpindex: u16,
    pub datagrams: Vec<NCMDatagram16>,
}

impl Default for NCMDatagramPointerTable {
    fn default() -> Self {
        NCMDatagramPointerTable {
            signature: u32::from_le_bytes(NDP16_SIGNATURE.try_into().unwrap()),
            length: 0,
            nextndpindex: 0,
            datagrams: Vec::<NCMDatagram16>::new(),
        }
    }
}


/// A USB stack error.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NCMError {
    // 
    TryFromSliceError,
    //header has created the wrong signature
    InvalidSignature,
    //
    ArrayError,

    //
    SizeError,

    RXError,
    TXError,
}

impl From<TryFromSliceError> for NCMError{
    fn from(_value: TryFromSliceError) -> Self {
        NCMError::TryFromSliceError
    }
}

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

pub trait ToBytes {
    fn conv_to_bytes(&self) -> Vec<u8>;
}

impl ToBytes for NCMDatagramPointerTable {
    fn conv_to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&self.signature.to_le_bytes());
        bytes.extend_from_slice(&self.length.to_le_bytes());
        bytes.extend_from_slice(&self.nextndpindex.to_le_bytes());

        self.datagrams.iter().for_each(|x| {
            bytes.extend_from_slice(x.index.to_le_bytes().as_slice());
            bytes.extend_from_slice(x.length.to_le_bytes().as_slice());
        });

        bytes
    }
}

impl ToBytes for NCMTransferHeader {
    fn conv_to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::<u8>::new();
        bytes.extend_from_slice(&self.signature.to_le_bytes());
        bytes.extend_from_slice(&self.headerlen.to_le_bytes());
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.extend_from_slice(&self.blocklen.to_le_bytes());
        bytes.extend_from_slice(&self.ndpindex.to_le_bytes());

        bytes
    }
}

impl TryInto<NCMTransferHeader> for &[u8] {
    type Error = NCMError;
    fn try_into(self) -> Result<NCMTransferHeader, Self::Error> {
        let signature = u32::from_le_bytes(self[0..4].try_into()?);
        if signature != u32::from_le_bytes(NTH16_SIGNATURE.try_into()?) {
            return Err(NCMError::InvalidSignature)
        } 


        Ok(NCMTransferHeader {
            signature: u32::from_le_bytes(self[0..4].try_into()?),
            headerlen: u16::from_le_bytes(self[4..6].try_into()?),
            sequence: u16::from_le_bytes(self[6..8].try_into()?),
            blocklen: u16::from_le_bytes(self[8..10].try_into()?),
            ndpindex: u16::from_le_bytes(self[10..12].try_into()?),
        })
    }
}

impl TryInto<NCMDatagramPointerTable> for &[u8] {
    type Error = NCMError;
    fn try_into(self) -> Result<NCMDatagramPointerTable, Self::Error> {
        let signature = u32::from_le_bytes(self[0..4].try_into()?);
        let length = u16::from_le_bytes(self[4..6].try_into()?);
        let nextndpindex = u16::from_le_bytes(self[6..8].try_into()?);

        if signature != u32::from_le_bytes(NDP16_SIGNATURE.try_into()?) {
            return Err(NCMError::InvalidSignature)
        }

        let datagrams = self[8..(length as usize)]
            .to_vec()
            .windows(4)
            .map(|win| NCMDatagram16 {
                index: u16::from_le_bytes(win[0..2].try_into().unwrap()),
                length: u16::from_le_bytes(win[2..4].try_into().unwrap()),
            })
            .collect::<Vec<NCMDatagram16>>();



        Ok(NCMDatagramPointerTable {
            signature,
            length,
            nextndpindex,
            datagrams,
        })
    }
}

pub struct UsbIp<'a, B>
where
    B: UsbBus,
{
    pub inner: CdcNcmClass<'a, B>,
}

impl<B> UsbIp<'_, B>
where
    B: UsbBus,
{
    /// Creates a new USB serial port with the provided UsbBus and 128 byte read/write buffers.
    pub fn new(alloc: &'_ UsbBusAllocator<B>) -> UsbIp<'_, B> {
        UsbIp {
            inner: CdcNcmClass::new(alloc),
        }
    }

    pub fn send_speed_notificaiton(&mut self) -> usb_device::Result<usize> where {
        let speedmsg: [u8; size_of::<CdcSpeedChangeMsg>()] =
            CdcSpeedChangeMsg::default().try_into().unwrap();
        self.inner.send_notification(speedmsg.as_slice())
    }
    pub fn send_connection_notificaiton(&mut self) -> usb_device::Result<usize> where {
        //update internal state as connected
        // self.ip_in.borrow_mut().set_connection_state(true);
        let conmsg: [u8; size_of::<CdcConnectionNotifyMsg>()] =
            CdcConnectionNotifyMsg::default().try_into().unwrap();
        self.inner.send_notification(conmsg.as_slice())
    }

    pub fn _ncm_writeall(&mut self) {
        //check if there are is any data in buf
    }

    //TODO: these fucntions handle exit from stall, need to ensure if we even need it.
    // pub fn ncm_indata(&mut self) -> Result<(), NCMError> {
    //     self.ip_in.send_state = NTBState::Empty;
    //     if self.ip_in.fill_state == NTBState::Ready{
    //     }
    //     Ok(())
    // }

    // pub fn ncm_outdata(&mut self) -> Result<(), NCMError>{
    //     Ok(())
    // }
}

impl<B> UsbClass<B> for UsbIp<'_, B>
where
    B: UsbBus,
{
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<(), UsbError> {
        self.inner.get_configuration_descriptors(writer)
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    // fn endpoint_in_complete(&mut self, addr: EndpointAddress) {
    //     if addr == self.inner.write_ep_address() {
    //         self.flush().ok();
    //     }
    // }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        self.inner.control_in(xfer);
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        self.inner.control_out(xfer);
    }
}
