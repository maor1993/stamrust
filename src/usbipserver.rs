use defmt::debug;
use usb_device::class_prelude::UsbBus;
use usb_device::device::UsbDevice;

use crate::ncm_netif::{BufState, SyncBuf};
use core::cell::RefMut;

use crate::cdc_ncm::EP_DATA_BUF_SIZE;
use crate::intf::{NCMDatagram16, NCMDatagramPointerTable, NCMTransferHeader, ToBytes, UsbIp};

#[derive(PartialEq)]
enum IpBootState {
    Speed,
    Notify,
    Normal,
}

enum IpRxState {
    AwaitHeader,
    AwaitNDP,
    LocateDataStart,
    CollectData,
    Reply,
}

enum IpTxState {
    Copy,
    Write,
}

// let mut rxstate = IpRxState::AwaitHeader;
// let mut txstate = IpTxState::Copy;
// let mut currheader: NCMTransferHeader;
// let mut currndp = NCMDatagramPointerTable::default();
// let mut currcnt = 0;
// let mut bytes_copied = 0;
// let mut gotfirstpacket = false;
// let mut txtransactioncnt = 0;
// let mut tcpserv = TcpServer::init_server();
// let mut txheader = NCMTransferHeader::default();
// let mut txdatagram = NCMDatagramPointerTable::default();
// let mut usbtxbuf: [u8; 2048] = [0u8; 2048];
// let mut usbmsgtotlen = 0;

pub struct UsbIpManager<'a, B: UsbBus> {
    ip_bus: UsbIp<'a, B>,
    usb_dev: UsbDevice<'a, B>,
    bootstate: IpBootState,
    rxstate: IpRxState,
    txstate: IpTxState,
    currheader: NCMTransferHeader,
    currndp: NCMDatagramPointerTable,
    currcnt: usize,
    bytes_copied: usize,
    gotfirstpacket: bool,
    txtransactioncnt: usize,
    txheader: NCMTransferHeader,
    txdatagram: NCMDatagramPointerTable,
    usbtxbuf: [u8; 2048],
    usbmsgtotlen: usize,
}

impl<'a, B: UsbBus> UsbIpManager<'a, B> {
    pub fn new(ip_bus: UsbIp<'a, B>, usb_dev: UsbDevice<'a, B>) -> UsbIpManager<'a, B> {
        UsbIpManager {
            ip_bus,
            usb_dev,
            bootstate: IpBootState::Speed,
            rxstate: IpRxState::AwaitHeader,
            txstate: IpTxState::Copy,
            currheader: NCMTransferHeader::default(),
            currndp: NCMDatagramPointerTable::default(),
            currcnt: 0,
            bytes_copied: 0,
            gotfirstpacket: false,
            txtransactioncnt: 9,
            txheader: NCMTransferHeader::default(),
            txdatagram: NCMDatagramPointerTable::default(),
            usbtxbuf: [0u8; 2048],
            usbmsgtotlen: 0,
        }
    }
    pub fn process_ndp(&mut self,usbbuf: &[u8],size:usize,mut rxbuf:RefMut<'_,SyncBuf>){
        self.currndp = usbbuf[(self.currheader.ndpindex as usize)..size]
        .try_into()
        .unwrap();
    // debug!("got ndp: {:?}", currndp);
    rxbuf.busy = BufState::Writing;
    if (self.currndp.datagrams[0].index as usize) < size {
        let diff = size - self.currndp.datagrams[0].index as usize;
        // the message starts in this packet, we can skip the locate state
        rxbuf.buf[0..diff].copy_from_slice(
            &usbbuf[self.currndp.datagrams[0].index as usize..size],
        );
        self.bytes_copied += diff;
        // debug!("copied {} bytes",bytes_copied);
        self.rxstate = IpRxState::CollectData;
    } else {
        self.currcnt = self.currndp.datagrams[0].index as usize - size; // start counting backwards until we reach the datagram
        self.rxstate = IpRxState::LocateDataStart;
    }
    }

    pub fn handle_data(&mut self, buffers: (RefMut<SyncBuf>, RefMut<SyncBuf>)) {
        let mut usbbuf: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
        let (mut rxbuf, mut txbuf) = buffers;

        if let IpTxState::Write = self.txstate {
            // we can only send chunks of 64 bytes, so we will incrementally walk the buffer
            let bytestocopy = (self.usbmsgtotlen - self.txtransactioncnt).min(EP_DATA_BUF_SIZE);
            let msg = &self.usbtxbuf[self.txtransactioncnt..self.txtransactioncnt + bytestocopy];

            if let Ok(size) = self.ip_bus.inner.write_packet(msg) {
                debug!("sent {} bytes", size);
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
                    self.txtransactioncnt += size;
                }
            }
        }

        if rxbuf.busy == BufState::Await {
            return;
        }

        if let Ok(size) = self.ip_bus.inner.read_packet(usbbuf.as_mut_slice()) {
            // debug!("got packet: {:x},size {}", rxbuf[0..size], size);
            match self.rxstate {
                IpRxState::AwaitHeader => {
                    if size < core::mem::size_of::<NCMTransferHeader>() {
                        return;
                    }

                    self.currheader = match usbbuf[0..size].try_into().ok() {
                        Some(x) => x,
                        None => return,
                    };
                    //there might be a big gap from the header and the ndp, if so, await until we reach the NDP location
                    if self.currheader.ndpindex as usize >= size{
                        self.currheader.ndpindex -= size as u16;
                        self.rxstate = IpRxState::AwaitNDP;
                    }
                    else{
                        self.process_ndp(&usbbuf, size, rxbuf);
                        self.rxstate = IpRxState::LocateDataStart;
                    }

                   
                }
                IpRxState::AwaitNDP => {  
                   if self.currheader.ndpindex < size as u16{
                        // the NDP is now in the buffer, send it to process
                        self.process_ndp(&usbbuf, size, rxbuf);
                        self.rxstate = IpRxState::LocateDataStart;
                   }
                   else{
                        self.currheader.ndpindex -= size as u16;
                   }
                }


                
                IpRxState::LocateDataStart => {
                    if self.currcnt <= size {
                        // the start of the datagram is located on this packet, start collecting to buffer
                        let diff = size - self.currcnt;
                        rxbuf.buf[0..diff].copy_from_slice(&usbbuf[self.currcnt..size]);
                        self.bytes_copied += diff;
                        self.rxstate = IpRxState::CollectData;
                    } else {
                        self.currcnt -= size;
                    }
                }
                IpRxState::CollectData => {
                    let bytes_to_copy =
                        (self.currndp.datagrams[0].length as usize - self.bytes_copied).min(size);
                    rxbuf.buf[self.bytes_copied..self.bytes_copied + bytes_to_copy]
                        .copy_from_slice(&usbbuf[0..bytes_to_copy]);
                    // debug!("copied {} bytes",bytes_copied);
                    self.bytes_copied += bytes_to_copy;
                    if self.currndp.datagrams[0].length as usize == self.bytes_copied {
                        // debug!(
                        //     "finished copying message: {:x}",
                        //     packetbuf.buf[0..currndp.datagrams[0].length as usize]
                        // );
                        rxbuf.len = self.bytes_copied;
                        self.bytes_copied = 0;
                        self.rxstate = IpRxState::Reply;
                        rxbuf.busy = BufState::Await;
                        self.gotfirstpacket = true;
                    }
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

                            self.usbtxbuf[0x0000..0x000C].copy_from_slice(headervec.as_slice());
                            self.usbtxbuf[0x0010..0x001C].copy_from_slice(datagramvec.as_slice());
                            //TODO: this is only correct for 1 datagram
                            self.usbtxbuf[0x0020..0x0020 + txbuf.len]
                                .copy_from_slice(txbuf.buf[0..txbuf.len].as_ref());

                            // info!("sending the following stream {:#02x}",usbtxbuf[0..usbmsgtotlen]);
                            self.txstate = IpTxState::Write;
                            txbuf.busy = BufState::Empty;
                            self.rxstate = IpRxState::AwaitHeader;
                        }
                    } else {
                        //missed this round.
                        self.rxstate = IpRxState::AwaitHeader;
                    }
                }
            }
        }
    }

    pub fn run_loop(&mut self, buffers: (RefMut<SyncBuf>, RefMut<SyncBuf>)) {
        if self.usb_dev.poll(&mut [&mut self.ip_bus.inner]) {
            match self.bootstate {
                IpBootState::Speed => {
                    if self.ip_bus.send_speed_notificaiton().is_ok() {
                        self.bootstate = IpBootState::Notify
                    }
                }
                IpBootState::Notify => {
                    if self.ip_bus.send_connection_notificaiton().is_ok() {
                        self.bootstate = IpBootState::Normal;
                        debug!("Sent notify!");
                    }
                }
                IpBootState::Normal => {
                    self.handle_data(buffers);
                }
            }
        }
    }
}
