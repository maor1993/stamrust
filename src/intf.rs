use core::fmt::Debug;

use smoltcp::phy::{Device,DeviceCapabilities, Medium, ChecksumCapabilities, Checksum};
use smoltcp::phy;
use defmt::println;

extern crate alloc;
use alloc::vec;

//example slice for a syn request:
// [
//             0x45, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0x64,
//             0xc0, 0xa8, 0x45, 0x01, 0x9f, 0x6e, 0x1b, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
//             0xa0, 0x02, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00                            
//             ]





pub struct UsbIpPhy{
    rx_buffer: [u8; 1536],
    tx_buffer: [u8; 1536],
}


impl UsbIpPhy {
    pub fn new() -> UsbIpPhy {
        UsbIpPhy {
            rx_buffer: [0; 1536],
            tx_buffer: [0; 1536],
        }
    }
}

pub struct UsbIpPhyRxToken<'a>(&'a mut [u8]);

impl<'a> phy::RxToken for UsbIpPhyRxToken<'a> {
    fn consume<R, F>(mut self, f: F) -> R
        where 
        F: FnOnce(&mut [u8]) -> R,
    {
        // TODO: receive packet into buffer

        self.0[0..48].copy_from_slice(&[
            0x45, 0x00, 0x00, 0x3c, 0x00, 0x00, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0x64,
            0xc0, 0xa8, 0x45, 0x01, 0x9f, 0x6e, 0x1b, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xa0, 0x02, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00                            
            ]);


        let result = f(self.0);

        // match result {
        //     Err(x) => rprintln!("failed with error {:?}",x),
        //     _ => ()
        // }
        // rprintln!("rx called");
        result
    }
}

pub struct UsbIpPhyTxToken<'a>(&'a mut [u8]);

impl<'a> phy::TxToken for UsbIpPhyTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
        where F: FnOnce(&mut [u8]) -> R
    {
        let result = f(&mut self.0[..len]);
        println!("tx called {}", len);
        // TODO: send packet out
        result
    }
}

//TODO: implment a miliseconds! counter for timestamp

impl Device for UsbIpPhy{
    fn transmit(&mut self, timestamp: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        Some(UsbIpPhyTxToken(&mut self.tx_buffer[..]))
    }
    fn receive(&mut self, timestamp: smoltcp::time::Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        Some((UsbIpPhyRxToken(&mut self.rx_buffer[..]),
        UsbIpPhyTxToken(&mut self.tx_buffer[..])))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut csum = ChecksumCapabilities::default();
        csum.ipv4 = Checksum::Tx;
        csum.tcp = Checksum::Tx;
        csum.udp = Checksum::Tx;
        csum.icmpv4 = Checksum::Tx;

        let mut cap = DeviceCapabilities::default();
        cap.medium= Medium::Ip;
        cap.max_transmission_unit= 1500-40;
        cap.max_burst_size= Some(1); //FIXME: needs to be no bigger than usb buffer?
        cap.checksum = csum;
        cap
        }
    type RxToken<'a> = UsbIpPhyRxToken<'a> where Self: 'a;
    type TxToken<'a> = UsbIpPhyTxToken<'a> where Self: 'a;
}
    
