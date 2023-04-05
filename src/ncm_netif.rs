use core::cell::RefMut;

use defmt::debug;
use smoltcp::phy;
use smoltcp::phy::{Checksum, ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use crate::intf::{UsbIpIn, UsbIpOut};

pub struct UsbIpPhy<'a> {
    pub tx: RefMut<'a,UsbIpIn>,
    pub rx: RefMut<'a,UsbIpOut>,
}

impl<'a> UsbIpPhy<'a> {
    pub fn new(tx: RefMut<'a,UsbIpIn>,rx: RefMut<'a,UsbIpOut>) -> UsbIpPhy<'a> {
        UsbIpPhy { tx ,rx}
    }
}

pub struct UsbIpPhyRxToken<'a>(&'a mut UsbIpOut);

impl<'a> phy::RxToken for UsbIpPhyRxToken<'a>

{
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // TODO: receive packet into buffer
        let mut buf: [u8; 1534] = [0; 1534];
        let len = self.0.ncm_getdatagram(&mut buf);
        debug!("Recieved {} bytes", len);
        let result = f(&mut buf);
        result
    }
}

pub struct UsbIpPhyTxToken<'a>(&'a mut UsbIpIn);
impl<'a, > phy::TxToken for UsbIpPhyTxToken<'a>
{
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let buf = self.0.ncm_allocdatagram(len).unwrap();
        let result = f(buf);
        debug!("tx called {}", len);
        debug!("{:?}", &buf[..len]);
        self.0.ncm_setdatagram().unwrap();
        result
    }
}

//TODO: implment a miliseconds! counter for timestamp

impl Device for UsbIpPhy<'_>
{
    fn transmit<'a>(&'_ mut self, _timestamp: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        Some(UsbIpPhyTxToken(&mut self.tx))
    }
    fn receive(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        Some((
            UsbIpPhyRxToken(&mut self.rx),
            UsbIpPhyTxToken(&mut self.tx),
        ))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut csum = ChecksumCapabilities::default();
        csum.ipv4 = Checksum::Tx;
        csum.tcp = Checksum::Tx;
        csum.udp = Checksum::Tx;
        csum.icmpv4 = Checksum::Tx;

        let mut cap = DeviceCapabilities::default();
        cap.medium = Medium::Ip;
        cap.max_transmission_unit = 1500 - 40;
        cap.max_burst_size = Some(1); //FIXME: needs to be no bigger than usb buffer?
        cap.checksum = csum;
        cap
    }
    type RxToken<'a> = UsbIpPhyRxToken<'a> where Self: 'a;
    type TxToken<'a> = UsbIpPhyTxToken<'a> where Self: 'a;
}
