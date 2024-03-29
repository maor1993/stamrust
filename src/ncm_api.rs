//NCM API
//processes ncm commands!

extern crate alloc;
use crate::cdc_ncm::EP_DATA_BUF_SIZE;
use crate::cdc_ncm::{NCM_MAX_IN_SIZE, NCM_MAX_OUT_SIZE};
use alloc::vec::Vec;
use core::array::TryFromSliceError;
use core::cmp::Ordering;
pub const NTH16_SIGNATURE: &[u8] = "NCMH".as_bytes();
pub const NDP16_SIGNATURE: &[u8] = "NCM0".as_bytes();

use crate::ncm_netif::{EthRingBuffers, MTU};
use crate::usbipserver::UsbRingBuffers;
use concurrent_queue::PushError;

use defmt::{debug, info, warn};

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
            ndpindex: 0x0c,
        }
    }
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct NCMDatagram16 {
    pub index: u16,
    pub length: u16,
}

#[repr(C)]
#[derive(Clone)]
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
            length: 0x10,
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

impl From<TryFromSliceError> for NCMError {
    fn from(_value: TryFromSliceError) -> Self {
        NCMError::TryFromSliceError
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
            return Err(NCMError::InvalidSignature);
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

        if signature != u32::from_le_bytes(NDP16_SIGNATURE.try_into()?) {
            return Err(NCMError::InvalidSignature);
        }

        let length = u16::from_le_bytes(self[4..6].try_into()?);
        let nextndpindex = u16::from_le_bytes(self[6..8].try_into()?);

        let datagrams = self[8..(length as usize)]
            .to_vec()
            .chunks(4)
            .map(|win| NCMDatagram16 {
                index: u16::from_le_bytes(win[0..2].try_into().unwrap()),
                length: u16::from_le_bytes(win[2..4].try_into().unwrap()),
            })
            .filter(|x| x.length != 0)
            .collect::<Vec<NCMDatagram16>>();

        Ok(NCMDatagramPointerTable {
            signature,
            length,
            nextndpindex,
            datagrams,
        })
    }
}

enum IpRxState {
    AwaitHeader,
    CopyEntireMsg,
}

enum IpTxState {
    Ready,
    Header,
    Sending,
    Zlp,
}

pub struct NcmApiManager {
    rxstate: IpRxState,
    txstate: IpTxState,
    currheader: NCMTransferHeader,
    currndp: NCMDatagramPointerTable,
    currcnt: usize,
    txtransactioncnt: usize,
    txheader: NCMTransferHeader,
    txdatagram: NCMDatagramPointerTable,
    ncmmsgtxbuf: [u8; NCM_MAX_OUT_SIZE],
    ncmmsgrxbuf: [u8; NCM_MAX_IN_SIZE],
    usbmsgtotlen: usize,
    currtxdatalen: usize,
    rxbufready: bool,
}

impl NcmApiManager {
    pub fn new() -> Self {
        NcmApiManager {
            rxstate: IpRxState::AwaitHeader,
            txstate: IpTxState::Ready,
            currheader: NCMTransferHeader::default(),
            currndp: NCMDatagramPointerTable::default(),
            currcnt: 0,
            txtransactioncnt: 0,
            txheader: NCMTransferHeader::default(),
            txdatagram: NCMDatagramPointerTable::default(),
            ncmmsgtxbuf: [0u8; NCM_MAX_OUT_SIZE],
            ncmmsgrxbuf: [0u8; NCM_MAX_IN_SIZE],
            usbmsgtotlen: 0,
            currtxdatalen: 0,
            rxbufready: false,
        }
    }

    pub fn process_ndp(&mut self) {
        self.currndp = self.ncmmsgrxbuf[(self.currheader.ndpindex as usize)..]
            .try_into()
            .unwrap();
    }

    fn restart_rx(&mut self) {
        self.rxstate = IpRxState::AwaitHeader;
        self.currcnt = 0;
    }

    pub fn process_messages(&mut self, eth_buffers: EthRingBuffers, usb_buffers: UsbRingBuffers) {
        let (rxq, txq) = eth_buffers;
        let (usbrxring, usbtxring) = usb_buffers;
        const TOTAL_HEADER_SIZE: usize = 0x001c;
        //TX HANDLING
        match self.txstate {
            IpTxState::Ready => {
                //we only need to copy the buffer
                if let Ok((msg_len, msg)) = txq.pop() {
                    debug!("sending {:02x}", msg[0..msg_len]);
                    //create a new datagram table entry for this message
                    self.currtxdatalen = msg_len;
                    self.txdatagram.datagrams.push(NCMDatagram16 {
                        index: TOTAL_HEADER_SIZE as u16,
                        length: msg_len as u16,
                    });
                    self.txdatagram.datagrams.push(NCMDatagram16 {
                        index: 0,
                        length: 0,
                    });

                    self.usbmsgtotlen = TOTAL_HEADER_SIZE + msg_len;
                    self.txheader.blocklen = self.usbmsgtotlen as u16;

                    let headervec = self.txheader.conv_to_bytes();
                    self.txheader.sequence = self.txheader.sequence.wrapping_add(1);
                    let datagramvec = self.txdatagram.conv_to_bytes();

                    self.ncmmsgtxbuf[0x0000..0x000C].copy_from_slice(headervec.as_slice());
                    self.ncmmsgtxbuf[0x000C..TOTAL_HEADER_SIZE]
                        .copy_from_slice(datagramvec.as_slice());
                    //TODO: this is only correct for 1 datagram
                    self.ncmmsgtxbuf[TOTAL_HEADER_SIZE..(TOTAL_HEADER_SIZE + msg_len)]
                        .copy_from_slice(msg[0..msg_len].as_ref());
                    debug!(
                        "sending the following stream {:02x}",
                        self.ncmmsgtxbuf[0..self.usbmsgtotlen]
                    );
                    if self.usbmsgtotlen <= EP_DATA_BUF_SIZE {
                        self.txstate = IpTxState::Header;
                    } else {
                        self.txstate = IpTxState::Sending;
                    }
                }
            }
            IpTxState::Header => {
                //send only the header.
                let mut msg: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
                msg[0..TOTAL_HEADER_SIZE].clone_from_slice(&self.ncmmsgtxbuf[0..TOTAL_HEADER_SIZE]);
                if let Ok(()) = usbtxring.push((TOTAL_HEADER_SIZE, msg)) {
                    if self.usbmsgtotlen == TOTAL_HEADER_SIZE {
                        self.txstate = IpTxState::Ready;
                    } else {
                        self.txtransactioncnt += TOTAL_HEADER_SIZE;
                        self.txstate = IpTxState::Sending;
                        debug!("sent header!");
                    }
                }
            }
            IpTxState::Sending => {
                // we can only send chunks of 64 bytes, so we will incrementally walk the buffer;
                let mut msg: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
                let bytestocopy = (self.usbmsgtotlen - self.txtransactioncnt).min(EP_DATA_BUF_SIZE);
                msg[0..bytestocopy].clone_from_slice(
                    &self.ncmmsgtxbuf[self.txtransactioncnt..self.txtransactioncnt + bytestocopy],
                );

                if let Ok(()) = usbtxring.push((bytestocopy, msg)) {
                    // info!("sent {} bytes", bytestocopy);
                    // info!("sent {:02x}", msg[0..bytestocopy]);
                    self.txtransactioncnt += bytestocopy;
                    //part of message sucesfully sent.
                    //increment the read pointer by size.

                    match self.txtransactioncnt.cmp(&self.usbmsgtotlen) {
                        Ordering::Equal => self.txstate = IpTxState::Zlp,
                        Ordering::Greater => panic!("ncm api tx sender sent too many bytes!"),
                        _ => (),
                    };
                } else {
                    // warn!("usb tx ring is full, waiting.");
                }
            }
            IpTxState::Zlp => {
                if (self.currtxdatalen % EP_DATA_BUF_SIZE) == 0 {
                    if let Ok(()) = usbtxring.push((0, [0u8; 64])) {
                        self.txdatagram.datagrams.clear();
                        self.txstate = IpTxState::Ready;
                        self.txtransactioncnt = 0;
                        info!("sent zlp!");
                    } else {
                        warn!("usb tx ring is full, waiting.");
                    }
                } else {
                    self.txdatagram.datagrams.clear(); 
                    self.txstate = IpTxState::Ready;
                    self.txtransactioncnt = 0;
                }
            }
        };
    
        // RX HANDLING
        for (size, usbbuf) in usbrxring.try_iter() {
            match self.rxstate {
                IpRxState::AwaitHeader => {
                    if size < core::mem::size_of::<NCMTransferHeader>() {
                        warn!("got unaligned ncm msg");
                        // info!("unaligned msg {:02x}", usbbuf[0..size]);
                        continue; //dont handle partial ncm headers
                    }
                    // attempt to parse the start of the buffer as a transfer header (by checking the signiture is correct)
                    self.currheader = match usbbuf[0..size].try_into().ok() {
                        Some(x) => x,
                        None => continue,
                    };
                    // info!("rx seq: {}",self.currheader.sequence);

                    //start copying towards the rx buffer
                    self.ncmmsgrxbuf[0..size].copy_from_slice(&usbbuf[0..size]);
                    self.currcnt += size;

                    //sanity check
                    if self.currheader.blocklen > NCM_MAX_IN_SIZE as u16 {
                        panic!("we received a message that is bigger than our buffer!");
                    }

                    self.rxstate = IpRxState::CopyEntireMsg;
                }
                IpRxState::CopyEntireMsg => {
                    let copysize = (self.currheader.blocklen as usize - self.currcnt).min(size);

                    self.ncmmsgrxbuf[self.currcnt..self.currcnt + copysize]
                        .copy_from_slice(&usbbuf[0..copysize]);

                    self.currcnt += copysize;

                    if self.currcnt == self.currheader.blocklen as usize {
                        self.restart_rx();
                        self.rxbufready = true
                    }
                }
            }
        }

        if self.rxbufready {
            self.process_ndp();
            const MAXSIZE: u16 = NCM_MAX_IN_SIZE as u16;
            debug!("processing {} datagrams", self.currndp.datagrams.len());
            self.currndp
                .datagrams
                .iter()
                .for_each(|dgram| match dgram.length {
                    0 => (),
                    1..=MAXSIZE => {
                        let idx_uz = dgram.index as usize;
                        let len_uz = dgram.length as usize;
                        let mut rxmsg: [u8; MTU] = [0u8; MTU];
                        rxmsg[0..len_uz]
                            .copy_from_slice(&self.ncmmsgrxbuf[idx_uz..idx_uz + len_uz]);
                        debug!("incoming {:02x}", rxmsg[0..len_uz]);
                        if let Err(x) = rxq.push((len_uz, rxmsg)) {
                            match x {
                                PushError::Full(_y) => warn!("rxq is full!"),
                                PushError::Closed(_y) => warn!("rxq is closed!"),
                            }
                        };
                    }
                    _ => panic!("Somehow we received a packet that is too big."),
                });
            self.rxbufready = false;
        }
    }
}
