use crate::cdc_ncm::{
    CdcNcmClass,NTBState,
    NCM_MAX_SEGMENT_SIZE, NDP16_SIGNATURE, NTH16_SIGNATURE,
};
use alloc::vec::Vec;
use core::array::TryFromSliceError;
use core::cell::RefCell;
use core::mem::size_of;
use defmt::debug;
use usb_device::bus::UsbBus;
use usb_device::class_prelude::*;
extern crate alloc;
const PAGE_SIZE: usize = 2048;

pub type NCMResult<T> = core::result::Result<T, NCMError>;


#[repr(C)]
#[derive(Debug, defmt::Format, Clone)]
pub struct NCMTransferHeader {
    pub signature: u32,
    pub headerlen: u16,
    pub sequence: u16,
    pub blocklen: u16,
    pub ndpidex: u16,
}

impl Default for NCMTransferHeader {
    fn default() -> Self {
        NCMTransferHeader {
            signature: u32::from_le_bytes(NTH16_SIGNATURE.try_into().unwrap()),
            headerlen: 0x000c,
            sequence: 0,
            blocklen: 0,
            ndpidex: 0x0010,
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
    //attempted to select unexistant page
    PageError,
    //header has created the wrong signature
    InvalidSignature,
    //
    ArrayError,

    //
    SizeError,

    RXError,
    TXError,
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

pub struct UsbIpIn {
    is_conn: bool,
    data: [PacketBuf; 2],
    max_size: usize,
    rem_size: usize,
    index: usize,
    page: u8,
    sequence: u16,
    send_state: NTBState,
    dgcount: u8,
    fill_state: NTBState,
}

impl UsbIpIn {
    fn ncm_sendntb(&mut self, page: u8) -> Result<&[u8], NCMError> {
        if page >= 2u8 {
            return Err(NCMError::PageError);
        }

        let currdataptr = &self.data[page as usize][0..];
        let mut datagrams = Vec::<NCMDatagram16>::new();
        let ptlen = size_of::<NCMDatagramPointerTable>()
            + (self.dgcount as usize) * size_of::<NCMDatagram16>();

        let nth = NCMTransferHeader {
            signature: u32::from_le_bytes(NTH16_SIGNATURE.try_into().unwrap()),
            headerlen: size_of::<NCMTransferHeader>() as u16,
            blocklen: (size_of::<NCMTransferHeader>() + self.index + ptlen) as u16,
            sequence: self.sequence,
            ndpidex: self.index as u16,
        };
        let dploc = self.get_dp_len_idx();

        for i in self.dgcount as usize..0 {
            datagrams.push(NCMDatagram16 {
                index: 0,
                length: u16::from_le_bytes(
                    currdataptr[dploc + 2 * i..dploc + 2 + 2 * i]
                        .try_into()
                        .unwrap(),
                ),
            })
        }
        // let mut prev_index;
        datagrams[0].index = size_of::<NCMTransferHeader>() as u16;

        for i in 1..self.dgcount as usize {
            datagrams[i].index = (datagrams[i - 1].index + datagrams[i - 1].length + 3) & 0xfffc;
        }

        datagrams.push(NCMDatagram16 {
            index: 0,
            length: 0,
        });

        let pt = NCMDatagramPointerTable {
            signature: u32::from_le_bytes(NDP16_SIGNATURE.try_into().unwrap()),
            length: ptlen as u16,
            nextndpindex: 0,
            datagrams,
        };

        //start copying to data buffer
        let ndppoint = self.index;

        self.data[page as usize][0..].copy_from_slice(nth.conv_to_bytes().as_slice());
        self.data[page as usize][ndppoint..].copy_from_slice(pt.conv_to_bytes().as_slice());

        //switch pages
        self.page = 1 - page;
        self.dgcount = 0;
        self.index = size_of::<NCMTransferHeader>();
        self.rem_size =
            self.max_size - size_of::<NCMTransferHeader>() - size_of::<NCMDatagramPointerTable>();
        self.fill_state = NTBState::Empty;
        self.send_state = NTBState::Transferring;

        self.sequence += 2;

        Ok(&self.data[page as usize])
    }

    pub fn ncm_setdatagram(&mut self) -> Result<(), NCMError> {
        if self.fill_state == NTBState::Processing {
            let page = self.page;

            if self.send_state != NTBState::Empty {
                self.fill_state = NTBState::Ready;
            } else {
                self.ncm_sendntb(page)?;
            }
        }
        Ok(())
    }

    fn get_dp_len_idx(&self) -> usize {
        size_of::<PacketBuf>() - (self.dgcount as usize * size_of::<u32>())
    }
    fn set_dp_len(&mut self, len: u16) {
        let dploc = self.get_dp_len_idx();
        self.data[self.page as usize][dploc..dploc + 2].copy_from_slice(&len.to_le_bytes());
    }

    pub fn ncm_allocdatagram(&mut self, len: usize) -> Result<&mut [u8], NCMError> {
        //ensure all criteria has been met before starting

        if (!self.is_conn)
            || (len >= NCM_MAX_SEGMENT_SIZE)
            || self.fill_state == NTBState::Processing
        {
            Err(NCMError::TXError)
        } else {
            // align allocated length to 32 bit boundary
            let wlen = (len + 3) & 0x0000_fffc;
            let addlen = wlen + size_of::<NCMDatagram16>();

            //update the in state to processing.
            self.fill_state = NTBState::Processing;
            let page = self.page as usize;

            //assuming there is enough room in buffer, push the new data page
            if addlen <= self.rem_size {
                self.dgcount += 1;
                self.set_dp_len(len as u16);

                let dataloc = self.index;
                self.index += wlen;
                self.rem_size -= addlen;

                Ok(&mut self.data[page][dataloc..]) //fixme: there is no way to ensure we're not overlapping other messages
            } else {
                Err(NCMError::ArrayError)
            }
        }
    }

    pub fn set_connection_state(&mut self, connected: bool) {
        self.is_conn = connected;
    }
}

pub struct UsbIpOut {
    data: [PacketBuf; 2],
    pt: Option<NCMDatagramPointerTable>,
    page: u8,
    dx: u8,
    state: [NTBState; 2],
}

impl UsbIpOut {
    pub fn updateptloc(&mut self, pt: NCMDatagramPointerTable) {
        self.pt = Some(pt)
    }

    pub fn ncm_getdatagram(&self, data: &mut [u8]) -> usize {
        // let page = self.page as usize;
        let mut len = 48;

        data[0..48].copy_from_slice(&[
            0x45, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 0xc0, 0xa8,
            0x45, 0x64, 0xc0, 0xa8, 0x45, 0x01, 0x9f, 0x6e, 0x1b, 0x12, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xa0, 0x02, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03,
            0x03, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);

        // match self.state[page]{
        //     NTBState::Ready => self.state[page] = NTBState::Processing,
        //     NTBState::Processing => {
        //         if let Some(x) = &mut self.pt{
        //             if(x.datagram[])
        //         }
        //     }

        // }

        // if self.state[page]== NTBState::Ready{
        //     self.state[page] = NTBState::Processing;
        // }
        // else{

        // }

        len
    }
}

type PacketBuf = [u8; PAGE_SIZE];

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
        bytes.extend_from_slice(&self.ndpidex.to_le_bytes());

        bytes
    }
}

impl TryInto<NCMTransferHeader> for &[u8] {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<NCMTransferHeader, Self::Error> {
        let signature = u32::from_le_bytes(self[0..4].try_into()?);
        if signature != u32::from_le_bytes(NTH16_SIGNATURE.try_into()?) {
            panic!("wrong signature");
        } 


        Ok(NCMTransferHeader {
            signature: u32::from_le_bytes(self[0..4].try_into()?),
            headerlen: u16::from_le_bytes(self[4..6].try_into()?),
            sequence: u16::from_le_bytes(self[6..8].try_into()?),
            blocklen: u16::from_le_bytes(self[8..10].try_into()?),
            ndpidex: u16::from_le_bytes(self[10..12].try_into()?),
        })
    }
}

impl TryInto<NCMDatagramPointerTable> for &[u8] {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<NCMDatagramPointerTable, Self::Error> {
        let signature = u32::from_le_bytes(self[0..4].try_into()?);
        let length = u16::from_le_bytes(self[4..6].try_into()?);
        let nextndpindex = u16::from_le_bytes(self[6..8].try_into()?);

        if signature != u32::from_le_bytes(NDP16_SIGNATURE.try_into()?) {
            panic!("wrong signature"); //FIXME: replace panic with custom error
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

impl Default for UsbIpOut {
    fn default() -> Self {
        UsbIpOut {
            data: [[0; PAGE_SIZE], [0; PAGE_SIZE]],
            pt: None,
            page: 0,
            dx: 0,
            state: [NTBState::default(), NTBState::default()],
        }
    }
}
impl Default for UsbIpIn {
    fn default() -> Self {
        UsbIpIn {
            is_conn: false,
            data: [[0; PAGE_SIZE], [0; PAGE_SIZE]],
            max_size: PAGE_SIZE,
            rem_size: PAGE_SIZE
                - size_of::<NCMTransferHeader>()
                - size_of::<NCMDatagramPointerTable>(),
            index: size_of::<NCMTransferHeader>(),
            page: 0,
            dgcount: 0,
            sequence: 0,
            send_state: NTBState::Ready,
            fill_state: NTBState::Empty,
        }
    }
}

pub struct UsbIp<'a, B>
where
    B: UsbBus,
{
    pub inner: CdcNcmClass<'a, B>,
    pub ip_in: RefCell<UsbIpIn>,
    pub ip_out: RefCell<UsbIpOut>,
}

impl<B> UsbIp<'_, B>
where
    B: UsbBus,
{
    /// Creates a new USB serial port with the provided UsbBus and 128 byte read/write buffers.
    pub fn new(alloc: &'_ UsbBusAllocator<B>) -> UsbIp<'_, B> {
        UsbIp {
            inner: CdcNcmClass::new(alloc),
            ip_in: RefCell::new(UsbIpIn::default()),
            ip_out: RefCell::new(UsbIpOut::default()),
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

    pub fn ncm_writeall(&mut self) {
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
