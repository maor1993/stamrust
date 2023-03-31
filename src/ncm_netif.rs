use smoltcp::phy;
use smoltcp::phy::{Checksum, ChecksumCapabilities, Device, DeviceCapabilities, Medium};
use defmt::debug;

pub struct UsbIpPhy {
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
        let result = f(self.0);
        result
    }
}

pub struct UsbIpPhyTxToken<'a>(&'a mut [u8]);

impl<'a> phy::TxToken for UsbIpPhyTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let result = f(&mut self.0[..len]);
        debug!("tx called {}", len);
        debug!("{:?}", &self.0[..len]);
        result
    }
}

//TODO: implment a miliseconds! counter for timestamp

impl Device for UsbIpPhy {
    fn transmit(&mut self, _timestamp: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        Some(UsbIpPhyTxToken(&mut self.tx_buffer[..]))
    }
    fn receive(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        Some((
            UsbIpPhyRxToken(&mut self.rx_buffer[..]),
            UsbIpPhyTxToken(&mut self.tx_buffer[..]),
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
