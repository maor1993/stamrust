use defmt::{debug, info, warn};
use usb_device::class_prelude::UsbBus;
use usb_device::device::UsbDevice;

use crate::ncm_netif::{BufState, SyncBuf};
use core::cell::RefMut;

use crate::cdc_ncm::EP_DATA_BUF_SIZE;
use crate::cdc_ncm::{NCM_MAX_IN_SIZE,NCM_MAX_OUT_SIZE};
use crate::intf::{NCMDatagram16, NCMDatagramPointerTable, NCMTransferHeader, ToBytes, UsbIp};

use concurrent_queue::ConcurrentQueue;

pub type Usbtransaciton = [u8; EP_DATA_BUF_SIZE];

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
    Reply,
}

enum IpTxState {
    Copy,
    Write,
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
    msgtxbuf: [u8; EP_DATA_BUF_SIZE],
    msghandled: bool,
}

impl<'a, B: UsbBus> UsbIpManager<'a, B> {
    pub fn new(ip_bus: UsbIp<'a, B>, usb_dev: UsbDevice<'a, B>) -> UsbIpManager<'a, B> {
        UsbIpManager {
            ip_bus,
            usb_dev,
            bootstate: UsbIpBootState::Speed,
            rxstate: IpRxState::AwaitHeader,
            txstate: IpTxState::Copy,
            currheader: NCMTransferHeader::default(),
            currndp: NCMDatagramPointerTable::default(),
            currcnt: 0,
            txtransactioncnt: 0,
            txheader: NCMTransferHeader::default(),
            txdatagram: NCMDatagramPointerTable::default(),
            ncmmsgtxbuf: [0u8; NCM_MAX_OUT_SIZE],
            ncmmsgrxbuf: [0u8; NCM_MAX_IN_SIZE],
            usbmsgtotlen: 0,
            msgtxbuf: [0u8; EP_DATA_BUF_SIZE],
            msghandled: true,
        }
    }
    pub fn process_ndp(&mut self){
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
            if let Ok(_size) = self.ip_bus.inner.read_packet(usbbuf.as_mut_slice()) {
                // debug!("usb buf receving {} bytes", size);
                usbrxring.push(usbbuf).unwrap();
            }
        } else {
            warn!("usb rx ring is full! flushing.");
            usbrxring.try_iter().for_each(|_x| ());
        }

        if self.msghandled & !usbtxring.is_empty() {
            self.msgtxbuf = usbtxring.pop().unwrap();
            self.msghandled = false
        }

        if !self.msghandled {
            if let Ok(_size) = self.ip_bus.inner.write_packet(&self.msgtxbuf[0..]) {
                info!(
                    "sending the following buffer to pc {:#02x}",
                    self.msgtxbuf[0..]
                );
                self.msghandled = true;
            } 
        }
    }

    pub fn run_loop(
        &mut self,
        buffers: (RefMut<SyncBuf>, RefMut<SyncBuf>),
        usbtxring: &mut ConcurrentQueue<Usbtransaciton>,
        usbrxring: &mut ConcurrentQueue<Usbtransaciton>,
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
                    self.process_usb(usbtxring, usbrxring);
                }
            }
        }
        self.process_messages(buffers, usbtxring, usbrxring);
    }

    fn restart_rx(&mut self){
        self.rxstate = IpRxState::AwaitHeader;
        self.currcnt = 0;
        self.txtransactioncnt = 0;
    }
    

    fn process_messages(
        &mut self,
        buffers: (RefMut<SyncBuf>, RefMut<SyncBuf>),
        usbtxring: &mut ConcurrentQueue<Usbtransaciton>,
        usbrxring: &mut ConcurrentQueue<Usbtransaciton>,
    ) {
        let (mut rxbuf, mut txbuf) = buffers;

        if let IpTxState::Write = self.txstate {
            // we can only send chunks of 64 bytes, so we will incrementally walk the buffer;
            let mut msg: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
            msg.clone_from_slice(
                &self.ncmmsgtxbuf[self.txtransactioncnt..self.txtransactioncnt + EP_DATA_BUF_SIZE],
            );
            if let Ok(()) = usbtxring.push(msg) {
                debug!("sent {} bytes", EP_DATA_BUF_SIZE);
                //part of message sucesfully sent.
                //increment the read pointer by size.
                if self.txtransactioncnt >= self.usbmsgtotlen {
                    //we've finished sending this message.
                    //after sending, increment message sequence
                    self.txheader.sequence += 1;
                    self.txdatagram.datagrams.clear();
                    self.txstate = IpTxState::Copy;
                    self.txtransactioncnt = 0;
                } else {
                    self.txtransactioncnt += EP_DATA_BUF_SIZE;
                }
            } else {
                warn!("usb tx ring is full, waiting.");
            }
        }

        let size = EP_DATA_BUF_SIZE;
        let usbbuf = match usbrxring.is_empty() {
            true => return,
            false => usbrxring.pop().unwrap(),
        };

        match self.rxstate {
            IpRxState::AwaitHeader => {
                if size < core::mem::size_of::<NCMTransferHeader>() {
                    panic!("How the fuck are we getting less than the header????");
                }

                // attempt to parse the start of the buffer as a transfer header (by checking the signiture is correct)
                self.currheader = match usbbuf[0..size].try_into().ok() {
                    Some(x) => x,
                    None => return,
                };

                //start copying towards the rx buffer 
                self.ncmmsgrxbuf[0..EP_DATA_BUF_SIZE].copy_from_slice(&usbbuf);
                self.currcnt += EP_DATA_BUF_SIZE;

                //sanity check
                if self.currheader.blocklen > NCM_MAX_IN_SIZE as u16 {
                    panic!("we received a message that is bigger than our buffer!");
                }

                self.rxstate = IpRxState::CopyEntireMsg;
               
            }
            IpRxState::CopyEntireMsg => {
                if self.currcnt >= self.currheader.blocklen as usize{
                    //we finished copying the entire message, time to start porcessing the transaction
                    self.rxstate = IpRxState::ProcessNDP;
                }
                else{
                    self.ncmmsgrxbuf[self.currcnt..self.currcnt+EP_DATA_BUF_SIZE].copy_from_slice(&usbbuf);
                    self.currcnt += EP_DATA_BUF_SIZE;
                }
            }

            IpRxState::ProcessNDP => {
                // the NDP is now in the buffer, send it to process
                self.process_ndp();
            }

            IpRxState::CopyToEthBuf => {
                const MAXSIZE:u16 = NCM_MAX_IN_SIZE as u16;
                info!("processing {} datagrams",self.currndp.datagrams.len());
                self.currndp.datagrams.iter().for_each(|dgram|{
                    match dgram.length{
                        0 =>  (),
                        1..=MAXSIZE => {
                            //since we know currently that only 1 dgram is support, we'll copy it directly
                            let idx_uz = dgram.index as usize;
                            let len_uz = dgram.length as usize;
                            rxbuf.buf[0..len_uz].copy_from_slice(&self.ncmmsgrxbuf[idx_uz..idx_uz+len_uz]);
                            self.rxstate = IpRxState::Reply;
                            rxbuf.len = len_uz;
                            rxbuf.busy = BufState::Await;
                        },
                        _ => panic!("Somehow we received a packet that is too big.")
                    }
                });
            }
                
            IpRxState::Reply => {
                if let BufState::Writing = txbuf.busy {
                    //we only need to copy the buffer
                    if let IpTxState::Copy = self.txstate {
                        //create a new datagram table entry for this message
                        self.txdatagram.datagrams.push(NCMDatagram16 {
                            index: 0x0020,
                            length: txbuf.len as u16,
                        });
                        self.usbmsgtotlen = 0x0020 + txbuf.len;
                        self.txheader.blocklen = self.usbmsgtotlen as u16;
                        self.txdatagram.length = 0x0010;

                        let headervec = self.txheader.conv_to_bytes();
                        let datagramvec = self.txdatagram.conv_to_bytes();

                        self.ncmmsgtxbuf[0x0000..0x000C].copy_from_slice(headervec.as_slice());
                        self.ncmmsgtxbuf[0x0010..0x001C].copy_from_slice(datagramvec.as_slice());
                        //TODO: this is only correct for 1 datagram
                        self.ncmmsgtxbuf[0x0020..0x0020 + txbuf.len]
                            .copy_from_slice(txbuf.buf[0..txbuf.len].as_ref());

                        debug!(
                            "sending the following stream {:#02x}",
                            self.ncmmsgtxbuf[0..self.usbmsgtotlen]
                        );
                        self.txstate = IpTxState::Write;
                        txbuf.busy = BufState::Empty;
                        self.rxstate = IpRxState::AwaitHeader;
                    }
                }
                else{
                    self.restart_rx();
                } 
            }
        }
    }

    // pub fn send
}
