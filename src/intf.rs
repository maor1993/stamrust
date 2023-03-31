use crate::cdc_ncm::{
    CdcNcmClass, NCMDatagram16, NCMDatagramPointerTable, NCMTransferHeader, NTBState,
    NCM_MAX_SEGMENT_SIZE, NDP16_SIGNATURE, NTH16_SIGNATURE,
};
use alloc::vec::Vec;
use core::array::TryFromSliceError;
use core::mem::size_of;
use defmt::debug;
use usb_device::bus::UsbBus;
use usb_device::class_prelude::*;
extern crate alloc;

pub type NCMResult<T> = core::result::Result<T, NCMError>;

const PAGE_SIZE: usize = 2048;

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

//example slice for a syn request:
// [
//             0x45, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0x64,
//             0xc0, 0xa8, 0x45, 0x01, 0x9f, 0x6e, 0x1b, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
//             0xa0, 0x02, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00
//             ]

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
    speedchange: NotifyHeader,
    speeddata: CdcSpeedChangeBody,
}

impl TryInto<[u8; size_of::<UsbIpNotify>()]> for UsbIpNotify {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<[u8; size_of::<UsbIpNotify>()], Self::Error> {
        const ARRLEN: usize = size_of::<UsbIpNotify>();
        let mut arr: [u8; ARRLEN] = [0; ARRLEN];
        arr[0] = self.speedchange.requestype;
        arr[1] = self.speedchange.notificationtype;
        arr[2..4].copy_from_slice(&self.speedchange.value.to_le_bytes());
        arr[4..6].copy_from_slice(&self.speedchange.index.to_le_bytes());
        arr[6..8].copy_from_slice(&self.speedchange.length.to_le_bytes());
        arr[8..12].copy_from_slice(&self.speeddata.bitrate_dl.to_le_bytes());
        arr[12..16].copy_from_slice(&self.speeddata.bitrate_ul.to_le_bytes());
        arr[16] = self.speedchange.requestype;
        arr[17] = self.speedchange.notificationtype;
        arr[18..20].copy_from_slice(&self.speedchange.value.to_le_bytes());
        arr[20..22].copy_from_slice(&self.speedchange.index.to_le_bytes());
        arr[22..24].copy_from_slice(&self.speedchange.length.to_le_bytes());
        Ok(arr)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UsbIpNotify {
    speedchange: NotifyHeader,
    speeddata: CdcSpeedChangeBody,
    connection: NotifyHeader,
}

struct UsbIpIn {
    data: [PacketBuf; 2],
    max_size: u32,
    rem_size: usize,
    index: u16,
    page: u8,
    sequence: u16,
    send_state: NTBState,
    dgcount: u8,
    fill_state: NTBState,
}

struct UsbIpOut {
    data: [PacketBuf; 2],
    pt: Option<NCMDatagramPointerTable>,
    page: u8,
    dx: u8,
    state: [NTBState; 2],
}

type PacketBuf = [u8; PAGE_SIZE];

impl TryInto<NCMTransferHeader> for &[u8] {
    type Error = TryFromSliceError;
    fn try_into(self) -> Result<NCMTransferHeader, Self::Error> {
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

        let datagrams = self[8..(length as usize)]
            .to_vec()
            .windows(4)
            .map(|win| {
                let dgram = NCMDatagram16 {
                    index: u16::from_le_bytes(win[0..2].try_into().unwrap()),
                    length: u16::from_le_bytes(win[2..4].try_into().unwrap()),
                };
                dgram
            })
            .collect::<Vec<NCMDatagram16>>();

        if signature != u32::from_le_bytes(NTH16_SIGNATURE.try_into()?) {
            panic!("wrong signature");
        }

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
            data: [[0; PAGE_SIZE], [0; PAGE_SIZE]],
            max_size: 0,
            rem_size: 0,
            index: 0,
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
    inner: CdcNcmClass<'a, B>,
    read_buf: [u8; 128],
    write_buf: [u8; 128],
    ip_in: UsbIpIn,
    ip_out: UsbIpOut,
    notify: UsbIpNotify,
}

impl<B> UsbIp<'_, B>
where
    B: UsbBus,
{
    /// Creates a new USB serial port with the provided UsbBus and 128 byte read/write buffers.
    pub fn new(alloc: &UsbBusAllocator<B>) -> UsbIp<'_, B> {
        UsbIp {
            inner: CdcNcmClass::new(alloc),
            read_buf: [0u8; 128],
            write_buf: [0u8; 128],
            ip_in: UsbIpIn::default(),
            ip_out: UsbIpOut::default(),
            notify: UsbIpNotify {
                speedchange: NotifyHeader {
                    requestype: 0xA1,
                    notificationtype: 0x2A,
                    value: 1,
                    index: 1,
                    length: size_of::<CdcSpeedChangeBody>() as u16,
                },
                speeddata: CdcSpeedChangeBody {
                    bitrate_dl: 10 * 1000000,
                    bitrate_ul: 10 * 1000000,
                },

                connection: NotifyHeader {
                    requestype: 0xA1,
                    notificationtype: 0x00,
                    value: 1,
                    index: 1,
                    length: 0,
                },
            },
        }
    }

    pub fn send_connection_notify(&mut self) {
        let data: [u8; size_of::<UsbIpNotify>()] = self.notify.try_into().unwrap();
        self.inner.send_notification(data.as_slice());
    }

    pub fn updateptloc(&mut self, pt: NCMDatagramPointerTable) {
        self.ip_out.pt = Some(pt)
    }

    fn ncm_getdatagram(&mut self, data: &[u8]) -> usize {
        // let page = self.ip_out.page as usize;
        let mut len = 0;
        // match self.ip_out.state[page]{
        //     NTBState::Ready => self.ip_out.state[page] = NTBState::Processing,
        //     NTBState::Processing => {
        //         if let Some(x) = &mut self.ip_out.pt{
        //             if(x.datagram[])
        //         }
        //     }

        // }

        // if self.ip_out.state[page]== NTBState::Ready{
        //     self.ip_out.state[page] = NTBState::Processing;
        // }
        // else{

        // }

        len
    }

    fn get_dp_len_idx(&self) -> usize {
        size_of::<PacketBuf>() + (self.ip_in.dgcount as usize * size_of::<u32>())
    }
    fn set_dp_len(&mut self, len: u16) {
        let dploc = self.get_dp_len_idx();
        self.ip_in.data[self.ip_in.page as usize][dploc..dploc + 2]
            .copy_from_slice(&len.to_le_bytes());
    }

    fn ncm_allocdatagram(&mut self, len: usize) -> Result<&[u8], NCMError> {
        //ensure all criteria has been met before starting
        if (self.notify.connection.value == 0)
            || (len >= NCM_MAX_SEGMENT_SIZE)
            || self.ip_in.fill_state == NTBState::Processing
        {
            Err(NCMError::TXError)
        } else {
            // create a rolling length
            let wlen = (len + 3) & 0x0000_fffc;
            let addlen = wlen + size_of::<NCMDatagram16>();

            //update the in state to processing.
            self.ip_in.fill_state = NTBState::Processing;
            let page = self.ip_in.page as usize;

            //assuming there is enough room in buffer, push the new data page
            if addlen <= self.ip_in.rem_size {
                self.ip_in.dgcount += 1;
                self.set_dp_len(len as u16);

                let dataloc = self.ip_in.index as usize;
                self.ip_in.index += wlen as u16;
                self.ip_in.rem_size -= addlen;

                Ok(&self.ip_in.data[page][dataloc..])
            } else {
                Err(NCMError::ArrayError)
            }
        }
    }

    fn ncm_setdatagram(&mut self) -> Result<(), NCMError> {
        if self.ip_in.fill_state == NTBState::Processing {
            let page = self.ip_in.page;

            if (self.ip_in.send_state != NTBState::Empty) {
                self.ip_in.fill_state = NTBState::Ready;
            } else {
                self.ncm_sendntb(page)?;
            }
        }
        Ok(())
    }

    fn ncm_sendntb(&mut self, page: u8) -> Result<(), NCMError> {
        if page >= 2u8 {
            return Err(NCMError::PageError);
        }

        let mut pt = NCMDatagramPointerTable {
            signature: u32::from_le_bytes(NDP16_SIGNATURE.try_into().unwrap()),
            length: (size_of::<NCMDatagramPointerTable>()
                + (self.ip_in.dgcount as usize) * size_of::<NCMDatagram16>())
                as u16,
            nextndpindex: 0,
            datagrams: Vec::<NCMDatagram16>::new(),
        };

        let mut nth = NCMTransferHeader {
            signature: u32::from_le_bytes(NTH16_SIGNATURE.try_into().unwrap()),
            headerlen: size_of::<NCMTransferHeader>() as u16,
            blocklen: size_of::<NCMTransferHeader>() as u16 + self.ip_in.index + pt.length,
            sequence: self.ip_in.sequence,
            ndpidex: self.ip_in.index,
        };

        // for i in [0..self.ip_in.dgcount] {
        //     pt.datagrams.push(NCMDatagram16 {
        //         index: i,
        //         length: (),
        //     })
        // }

        Ok(())
    }
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
        self.read_buf.fill(0);
        self.write_buf.fill(0);
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

