use defmt::println;
use smoltcp::phy::{self, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
extern crate alloc;
use core::cell::RefCell;

const MTU: usize = 1536;

#[derive(PartialEq)]
pub enum BufState {
    Empty,
    Writing,
    Await,
}

pub struct SyncBuf {
    pub busy: BufState,
    pub len: usize,
    pub buf: [u8; MTU],
}

impl Default for SyncBuf {
    fn default() -> Self {
        Self {
            busy: BufState::Empty,
            len : 0,
            buf: [0u8; MTU],
        }
    }
}
pub struct StmPhy {
    pub rxbuf: RefCell<SyncBuf>,
    pub txbuf: RefCell<SyncBuf>,
}

impl StmPhy {
    pub fn new() -> StmPhy {
        StmPhy {
            rxbuf: RefCell::<SyncBuf>::new(SyncBuf::default()),
            txbuf: RefCell::<SyncBuf>::new(SyncBuf::default()),
        }
    }
}

impl phy::Device for StmPhy {
    type RxToken<'a> = StmPhyRxToken<'a> where Self: 'a;
    type TxToken<'a> = StmPhyTxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let rxbuf = self.rxbuf.get_mut();
        let txbuf = self.txbuf.get_mut();

        match rxbuf.busy {
            BufState::Await => Some((StmPhyRxToken(rxbuf), StmPhyTxToken(txbuf))),
            _ => None,
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let txbuf = self.txbuf.get_mut();
        match txbuf.busy {
            BufState::Writing => None,
            _ => Some(StmPhyTxToken(txbuf)),
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = Some(1);
        caps.medium = Medium::Ethernet;
        caps
    }
}

pub struct StmPhyRxToken<'a>(&'a mut SyncBuf);

impl<'a> phy::RxToken for StmPhyRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // TODO: receive packet into buffer

        let result = f(&mut self.0.buf[0..self.0.len]);
        println!("rx called: {:?}",self.0.buf[0..self.0.len]);
        self.0.busy = BufState::Empty;
        result
    }
}

pub struct StmPhyTxToken<'a>(&'a mut SyncBuf);

impl<'a> phy::TxToken for StmPhyTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let result = f(&mut self.0.buf[..len]);
        println!("tx called {}", len);
        //update buffer with new pending packet
        self.0.len = len;
        self.0.busy = BufState::Writing;
        // TODO: send packet out
        result
    }
}
