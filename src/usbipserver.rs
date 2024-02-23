use core::mem::size_of;
use defmt::{debug, info, warn};
use usb_device::class_prelude::{UsbBus, UsbBusAllocator};
use usb_device::prelude::*;

use crate::cdc_ncm::{CdcConnectionNotifyMsg, CdcSpeedChangeMsg};
use crate::cdc_ncm::{CdcNcmClass, EP_DATA_BUF_SIZE};
pub type Usbtransaciton = (usize, [u8; EP_DATA_BUF_SIZE]);
use crate::cdc_ncm::{CDC_SUBCLASS_NCM, USB_CLASS_CDC};
use concurrent_queue::ConcurrentQueue;

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

pub struct UsbIpManager<'a, B: UsbBus> {
    ncm_dev: CdcNcmClass<'a, B>,
    usb_dev: UsbDevice<'a, B>,
    bootstate: UsbIpBootState,
    currtxbuf: (usize, [u8; EP_DATA_BUF_SIZE]),
    txq: ConcurrentQueue<Usbtransaciton>,
    rxq: ConcurrentQueue<Usbtransaciton>,
    msghandled: bool,
}

impl<'a, B: UsbBus> UsbIpManager<'a, B> {
    pub fn new(usb_alloc: &'a UsbBusAllocator<B>) -> UsbIpManager<'a, B> {
        let ncm_dev = CdcNcmClass::new(usb_alloc);
        let usb_dev = UsbDeviceBuilder::new(usb_alloc, UsbVidPid(0x0483, 0xffff))
            .strings(&[StringDescriptors::new(LangID::EN_US)
                .manufacturer("STMicroelectronics")
                .product("IP over USB Demonstrator")
                .serial_number("test")])
            .expect("failed to create strings")
            .device_class(USB_CLASS_CDC)
            .device_sub_class(CDC_SUBCLASS_NCM)
            .composite_with_iads()
            .build();

        UsbIpManager {
            ncm_dev,
            usb_dev,
            bootstate: UsbIpBootState::Speed,
            currtxbuf: (0, [0u8; EP_DATA_BUF_SIZE]),
            msghandled: true,
            txq: ConcurrentQueue::<Usbtransaciton>::bounded(8),
            rxq: ConcurrentQueue::<Usbtransaciton>::bounded(4),
        }
    }
    fn process_usb(&mut self) {
        if !self.rxq.is_full() {
            let mut usbbuf: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
            if let Ok(size) = self.ncm_dev.read_packet(usbbuf.as_mut_slice()) {
                // debug!("usb buf receving {} bytes", size);
                self.rxq.push((size, usbbuf)).unwrap();
            }
        } else {
            warn!("usb rx ring is full! flushing.");
            self.rxq.try_iter().for_each(|_x| ());
        }

        if self.msghandled {
            if let Ok(x) = self.txq.pop() {
                self.currtxbuf = x;
                self.msghandled = false;
            }
        } else {
            let (size, msg) = self.currtxbuf;
            if let Ok(_size) = self.ncm_dev.write_packet(&msg[0..size]) {
                debug!("sending the following buffer to pc {:#02x}", msg[0..size]);
                self.msghandled = true;
            }
        }
    }

    pub fn run_loop(&mut self) {
        self.poll_usb();
        match self.bootstate {
            UsbIpBootState::Speed => {
                if self.send_speed_notificaiton().is_ok() {
                    self.bootstate = UsbIpBootState::Notify
                }
            }
            UsbIpBootState::Notify => {
                if self.send_connection_notificaiton().is_ok() {
                    self.bootstate = UsbIpBootState::Normal;
                    debug!("Sent notify!");
                }
            }
            UsbIpBootState::Normal => {
                self.process_usb();
            }
        }
    }
    fn poll_usb(&mut self) -> bool{
        self.usb_dev.poll(&mut [&mut self.ncm_dev])
    }

    pub fn get_bufs(&mut self) -> UsbRingBuffers {
        (&mut self.rxq, &mut self.txq)
    }

    fn send_speed_notificaiton(&mut self) -> usb_device::Result<usize> {
        let speedmsg: [u8; size_of::<CdcSpeedChangeMsg>()] =
            CdcSpeedChangeMsg::default().try_into().unwrap();
        self.ncm_dev.send_notification(speedmsg.as_slice())
    }
    fn send_connection_notificaiton(&mut self) -> usb_device::Result<usize> {
        //update internal state as connected
        // self.ip_in.borrow_mut().set_connection_state(true);
        let conmsg: [u8; size_of::<CdcConnectionNotifyMsg>()] =
            CdcConnectionNotifyMsg::default().try_into().unwrap();
        self.ncm_dev.send_notification(conmsg.as_slice())
    }
}
