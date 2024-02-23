
use defmt::error;
use smoltcp::phy::{self, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use smoltcp::wire::IPV4_MIN_MTU;
extern crate alloc;
use concurrent_queue::ConcurrentQueue;
pub const MTU: usize = IPV4_MIN_MTU;
const MAX_QUEUE_SIZE: usize = 2;

pub type Ethmsg = (usize,[u8;MTU]);
pub type  EthRingBuffers<'a> = (&'a mut ConcurrentQueue<Ethmsg>,&'a mut ConcurrentQueue<Ethmsg>);

pub struct StmPhy {
    pub rxq: ConcurrentQueue::<Ethmsg>,
    pub txq: ConcurrentQueue::<Ethmsg>,
}

impl StmPhy {
    pub fn new() -> StmPhy {
        StmPhy {
            rxq: ConcurrentQueue::<Ethmsg>::bounded(MAX_QUEUE_SIZE),
            txq: ConcurrentQueue::<Ethmsg>::bounded(MAX_QUEUE_SIZE),
        }
    }
}

impl phy::Device for StmPhy {
    type RxToken<'a> = StmPhyRxToken<'a> where Self: 'a;
    type TxToken<'a> = StmPhyTxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if !self.rxq.is_empty(){
           return Some((StmPhyRxToken(&mut self.rxq), StmPhyTxToken(&mut self.txq)))
        }
        None
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if !self.txq.is_full(){
            return Some(StmPhyTxToken(&mut self.txq));
        }
        None
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = Some(1);
        caps.medium = Medium::Ethernet;
        caps
    }
}

pub struct StmPhyRxToken<'a>(&'a mut ConcurrentQueue<Ethmsg>);

impl<'a> phy::RxToken for StmPhyRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        if let Ok(mut x) = self.0.pop(){
            let result: R = f(&mut x.1);
            result
        }
        else{
            panic!("RX token called but queue was empty");
        }
        
       
    }
}

pub struct StmPhyTxToken<'a>(&'a mut ConcurrentQueue<Ethmsg>);

impl<'a> phy::TxToken for StmPhyTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut output = [0u8;MTU];
        let result = f(&mut output[0..len]);
        if let Err(_x) =self.0.push((len,output)) {
            error!("overloaded ethernet tx buf, dropped packet!");
        }
        //update buffer with new pending packet
        result
    }
}
