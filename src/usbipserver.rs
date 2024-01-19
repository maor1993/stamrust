use defmt::{debug, warn};
use usb_device::class_prelude::{UsbBus, UsbBusAllocator};
use usb_device::prelude::*;

use crate::ncm_netif::{EthRingBuffers, MTU};

use crate::cdc_ncm::EP_DATA_BUF_SIZE;
use crate::intf::UsbIp;
pub type Usbtransaciton = (usize, [u8; EP_DATA_BUF_SIZE]);
use crate::cdc_ncm::{CDC_SUBCLASS_NCM, USB_CLASS_CDC};
use concurrent_queue::{ConcurrentQueue, PushError};

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
    ip_bus: UsbIp<'a, B>,
    usb_dev: UsbDevice<'a, B>,
    bootstate: UsbIpBootState,
    currtxbuf: (usize, [u8; EP_DATA_BUF_SIZE]),
    msghandled: bool,
}

impl<'a, B: UsbBus> UsbIpManager<'a, B> {
    pub fn new(usb_alloc: &'a UsbBusAllocator<B>) -> UsbIpManager<'a, B> {
        let ip_bus = UsbIp::new(usb_alloc);
        let usb_dev = UsbDeviceBuilder::new(usb_alloc, UsbVidPid(0x0483, 0xffff))
            .device_class(USB_CLASS_CDC)
            .device_sub_class(CDC_SUBCLASS_NCM)
            .build();

        UsbIpManager {
            ip_bus,
            usb_dev,
            bootstate: UsbIpBootState::Speed,
            currtxbuf: (0, [0u8; EP_DATA_BUF_SIZE]),
            msghandled: true,
        }
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
                usbrxring.push((size, usbbuf)).unwrap();
            }
        } else {
            warn!("usb rx ring is full! flushing.");
            usbrxring.try_iter().for_each(|_x| ());
        }

        if self.msghandled {
            if let Ok(x) = usbtxring.pop() {
                self.currtxbuf = x;
                self.msghandled = false;
            }
        } else {
            let (size, msg) = self.currtxbuf;
            if let Ok(_size) = self.ip_bus.inner.write_packet(&msg[0..size]) {
                debug!("sending the following buffer to pc {:#02x}", msg[0..size]);
                self.msghandled = true;
            }
        }
    }

    pub fn run_loop(&mut self, usbring: UsbRingBuffers) {
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
    }

    // pub fn send
}
