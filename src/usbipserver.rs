use defmt::{debug, warn};
use usb_device::class_prelude::UsbBus;
use usb_device::device::UsbDevice;

use crate::ncm_netif::{EthRingBuffers, MTU};

use crate::cdc_ncm::EP_DATA_BUF_SIZE;
use crate::cdc_ncm::{NCM_MAX_IN_SIZE, NCM_MAX_OUT_SIZE};
use crate::intf::{NCMDatagram16, NCMDatagramPointerTable, NCMTransferHeader, ToBytes, UsbIp};

use concurrent_queue::{ConcurrentQueue, PushError};

pub type Usbtransaciton = (usize,[u8; EP_DATA_BUF_SIZE]);

pub type UsbRingBuffers<'a> = (
    &'a mut ConcurrentQueue<Usbtransaciton>,
    &'a mut ConcurrentQueue<Usbtransaciton>,
);

#[derive(PartialEq)]
enum UsbIpBootState {
    Speed,
    Notify,
    Normal,
}

enum IpRxState {
    AwaitHeader,
    CopyEntireMsg,
    ProcessNDP,
    CopyToEthBuf,
}

enum IpTxState {
    Ready,
    Sending,
}

pub struct UsbIpManager<'a, B: UsbBus> {
    ip_bus: UsbIp<'a, B>,
    usb_dev: UsbDevice<'a, B>,
    bootstate: UsbIpBootState,
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
    currtxbuf: (usize,[u8; EP_DATA_BUF_SIZE]),
    msghandled: bool,
}

impl<'a, B: UsbBus> UsbIpManager<'a, B> {
    pub fn new(ip_bus: UsbIp<'a, B>, usb_dev: UsbDevice<'a, B>) -> UsbIpManager<'a, B> {
        UsbIpManager {
            ip_bus,
            usb_dev,
            bootstate: UsbIpBootState::Speed,
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
            currtxbuf: (0,[0u8; EP_DATA_BUF_SIZE]),
            msghandled: true,
        }
    }
    pub fn process_ndp(&mut self) {
        self.currndp = self.ncmmsgrxbuf[(self.currheader.ndpindex as usize)..]
            .try_into()
            .unwrap();
        self.rxstate = IpRxState::CopyToEthBuf;
    }
    fn process_usb(
        &mut self,
        usbtxring: &mut ConcurrentQueue<Usbtransaciton>,
        usbrxring: &mut ConcurrentQueue<Usbtransaciton>,
    ) {
        if !usbrxring.is_full() {
            let mut usbbuf: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
            if let Ok(size) = self.ip_bus.inner.read_packet(usbbuf.as_mut_slice()) {
                // debug!("usb buf receving {} bytes", size);
                usbrxring.push((size,usbbuf)).unwrap();
            }
        } else {
            warn!("usb rx ring is full! flushing.");
            usbrxring.try_iter().for_each(|_x| ());
        }

        if self.msghandled{
            if let Ok(x) = usbtxring.pop(){
                self.currtxbuf = x;
                self.msghandled = false;
            }
        }
        else 
        {
            let (size,msg) = self.currtxbuf;
            if let Ok(_size) = self.ip_bus.inner.write_packet(&msg[0..size]) {
                debug!(
                    "sending the following buffer to pc {:#02x}",
                    msg[0..size]
                );
                self.msghandled = true;
            }
        }
    }

    pub fn run_loop(
        &mut self,
        ethring: EthRingBuffers,
        usbring: UsbRingBuffers,
    ) {
        if self.usb_dev.poll(&mut [&mut self.ip_bus.inner]) {
            match self.bootstate {
                UsbIpBootState::Speed => {
                    if self.ip_bus.send_speed_notificaiton().is_ok() {
                        self.bootstate = UsbIpBootState::Notify
                    }
                }
                UsbIpBootState::Notify => {
                    if self.ip_bus.send_connection_notificaiton().is_ok() {
                        self.bootstate = UsbIpBootState::Normal;
                        debug!("Sent notify!");
                    }
                }
                UsbIpBootState::Normal => {
                    self.process_usb(usbring.0, usbring.1);
                }
            }
        }
        self.process_messages(ethring, usbring.0, usbring.1);
    }

    fn restart_rx(&mut self) {
        self.rxstate = IpRxState::AwaitHeader;
        self.currcnt = 0;
        self.txtransactioncnt = 0;
    }

    fn process_messages(
        &mut self,
        buffers: EthRingBuffers,
        usbtxring: &mut ConcurrentQueue<Usbtransaciton>,
        usbrxring: &mut ConcurrentQueue<Usbtransaciton>,
    ) {
        let (rxq, txq) = buffers;

        //TX HANDLING
        match self.txstate {
            IpTxState::Ready => {
                //we only need to copy the buffer
                if let Ok((msg_len, msg)) = txq.pop() {
                    //create a new datagram table entry for this message
                    self.txdatagram.datagrams.push(NCMDatagram16 {
                        index: 0x0020,
                        length: msg_len as u16,
                    });
                    self.usbmsgtotlen = 0x0020 + msg_len;
                    self.txheader.blocklen = self.usbmsgtotlen as u16;
                    self.txdatagram.length = 0x0010;

                    let headervec = self.txheader.conv_to_bytes();
                    let datagramvec = self.txdatagram.conv_to_bytes();

                    self.ncmmsgtxbuf[0x0000..0x000C].copy_from_slice(headervec.as_slice());
                    self.ncmmsgtxbuf[0x0010..0x001C].copy_from_slice(datagramvec.as_slice());
                    //TODO: this is only correct for 1 datagram
                    self.ncmmsgtxbuf[0x0020..0x0020 + msg_len]
                        .copy_from_slice(msg[0..msg_len].as_ref());
                    debug!(
                        "sending the following stream {:#02x}",
                        self.ncmmsgtxbuf[0..self.usbmsgtotlen]
                    );
                    self.txstate = IpTxState::Sending;
                }
            }
            IpTxState::Sending => {
                // we can only send chunks of 64 bytes, so we will incrementally walk the buffer;
                let mut msg: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
                let bytestocopy = (self.usbmsgtotlen -self.txtransactioncnt).min(EP_DATA_BUF_SIZE);
                msg[0..bytestocopy].clone_from_slice(
                    &self.ncmmsgtxbuf
                        [self.txtransactioncnt..self.txtransactioncnt + bytestocopy],
                );
                
                if let Ok(()) = usbtxring.push((bytestocopy,msg)) {
                    debug!("sent {} bytes", bytestocopy);
                    self.txtransactioncnt += bytestocopy;
                    //part of message sucesfully sent.
                    //increment the read pointer by size.
                    if self.txtransactioncnt >= self.usbmsgtotlen {
                        //we've finished sending this message.
                        //after sending, increment message sequence
                        self.txheader.sequence += 1;
                        self.txdatagram.datagrams.clear(); // TODO: this is stopping us from sending more than 1 dgram
                        self.txstate = IpTxState::Ready;
                        self.txtransactioncnt = 0;
                    } 
                } else {
                    warn!("usb tx ring is full, waiting.");
                }
            }
        };

        // RX HANDLING
        let (size,usbbuf) = match usbrxring.is_empty() {
            true => return,
            false => usbrxring.pop().unwrap(),
        };

        match self.rxstate {
            IpRxState::AwaitHeader => {
                if size < core::mem::size_of::<NCMTransferHeader>() {
                    return //dont handle partial ncm headers
                }
                // attempt to parse the start of the buffer as a transfer header (by checking the signiture is correct)
                self.currheader = match usbbuf[0..size].try_into().ok() {
                    Some(x) => x,
                    None => return,
                };

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
                if self.currcnt >= self.currheader.blocklen as usize {
                    //we finished copying the entire message, time to start porcessing the transaction
                    self.rxstate = IpRxState::ProcessNDP;
                } else {
                    self.ncmmsgrxbuf[self.currcnt..self.currcnt + size]
                        .copy_from_slice(&usbbuf[0..size]);
                    self.currcnt += size;
                }
            }

            IpRxState::ProcessNDP => {
                // the NDP is now in the buffer, send it to process
                self.process_ndp();
            }

            IpRxState::CopyToEthBuf => {
                const MAXSIZE: u16 = NCM_MAX_IN_SIZE as u16;
                debug!("processing {} datagrams", self.currndp.datagrams.len());
                self.currndp.datagrams.iter().for_each(|dgram| {
                    match dgram.length {
                        0 => (),
                        1..=MAXSIZE => {
                            //since we know currently that only 1 dgram is support, we'll copy it directly
                            let idx_uz = dgram.index as usize;
                            let len_uz = dgram.length as usize;
                            let mut rxmsg: [u8; MTU] = [0u8; MTU];
                            rxmsg[0..len_uz].copy_from_slice(&self.ncmmsgrxbuf[idx_uz..idx_uz + len_uz]);
                            
                            if let Err(x) =  rxq.push((len_uz, rxmsg)){
                                match x {
                                    PushError::Full(_y) => warn!("rxq is full!"),
                                    PushError::Closed(_y) => warn!("rxq is closed!")
                                }
                            }; //TODO: now that we did this we can push more than one message!
                            
                            
                        }
                        _ => panic!("Somehow we received a packet that is too big."),
                    }
                });
                self.restart_rx();
            }
        }
    }

    // pub fn send
}
