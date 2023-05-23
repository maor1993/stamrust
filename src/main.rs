#![no_std]
#![no_main]

use core::borrow::BorrowMut;

use cortex_m::interrupt::Mutex;
//runtime
use cortex_m_rt::entry;
use defmt::debug;
use defmt_rtt as _;
use embedded_alloc::Heap;
use panic_probe as _;
// hal
use stm32l4xx_hal::usb::{Peripheral, UsbBus};
use stm32l4xx_hal::{prelude::*, stm32};
use usb_device::prelude::*;

//app
mod cdc_ncm;
use cdc_ncm::{CDC_SUBCLASS_NCM, USB_CLASS_CDC};
mod intf;
mod ncm_netif;
mod server;
use intf::UsbIp;
use server::TcpServer;

use crate::cdc_ncm::{
    NCMDatagramPointerTable, NCMTransferHeader, EP_DATA_BUF_SIZE, NCM_MAX_OUT_SIZE,
};

#[global_allocator]
static HEAP: Heap = Heap::empty();

fn enable_crs() {
    let rcc = unsafe { &(*stm32::RCC::ptr()) };
    rcc.apb1enr1.modify(|_, w| w.crsen().set_bit());
    let crs = unsafe { &(*stm32::CRS::ptr()) };
    // Initialize clock recovery
    // Set autotrim enabled.
    crs.cr.modify(|_, w| w.autotrimen().set_bit());
    // Enable CR
    crs.cr.modify(|_, w| w.cen().set_bit());
}

/// Enables VddUSB power supply
fn enable_usb_pwr() {
    // Enable PWR peripheral
    let rcc = unsafe { &(*stm32::RCC::ptr()) };
    rcc.apb1enr1.modify(|_, w| w.pwren().set_bit());

    // Enable VddUSB
    let pwr = unsafe { &*stm32::PWR::ptr() };
    pwr.cr2.modify(|_, w| w.usv().set_bit());
}

fn init_heap() {
    use core::mem::MaybeUninit;
    const HEAP_SIZE: usize = 8192;
    #[link_section = ".ram2bss"]
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
    unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
}

enum IpBootState {
    Speed,
    Notify,
    Normal,
}

enum IpRxState {
    AwaitHeader,
    LocateDataStart,
    CollectData,
    Reply,
}

#[entry]
fn main() -> ! {
    init_heap();
    let dp = stm32::Peripherals::take().unwrap();

    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();
    let mut pwr = dp.PWR.constrain(&mut rcc.apb1r1);

    let _clocks = rcc
        .cfgr
        .hsi48(true)
        .sysclk(80.MHz())
        .freeze(&mut flash.acr, &mut pwr);

    enable_crs();

    // disable Vddusb power isolation
    enable_usb_pwr();

    // Configure the on-board LED (LD3, green)
    let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);
    let mut led = gpioa
        .pa9
        .into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper);
    led.set_low(); // Turn off

    let usb = Peripheral {
        usb: dp.USB,
        pin_dm: gpioa
            .pa11
            .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
        pin_dp: gpioa
            .pa12
            .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
    };
    let usb_bus = UsbBus::new(usb);

    let mut ip = UsbIp::new(&usb_bus);

    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x0483, 0xffff))
        .manufacturer("STMicroelectronics")
        .product("IP over USB Demonstrator")
        .serial_number("test")
        .device_release(0x0100)
        .device_class(USB_CLASS_CDC)
        .build();

    debug!("starting server...");
    let mut bootstate = IpBootState::Speed;
    let mut rxstate = IpRxState::AwaitHeader;
    let mut currheader = NCMTransferHeader::default();
    let mut currndp = NCMDatagramPointerTable::default();
    let mut currcnt = 0;
    let mut packetbuf = [0u8; NCM_MAX_OUT_SIZE];
    let mut bytes_copied = 0;
    // let mut tcpserv = TcpServer::init_server(ip.ip_in.borrow_mut(),ip.ip_out.borrow_mut());
    loop {
        if usb_dev.poll(&mut [&mut ip.inner]) {
            match bootstate {
                IpBootState::Speed => {
                    if ip.send_speed_notificaiton().is_ok() {
                        bootstate = IpBootState::Notify
                    }
                }
                IpBootState::Notify => {
                    if ip.send_connection_notificaiton().is_ok() {
                        bootstate = IpBootState::Normal;
                        debug!("Sent notify!");
                    }
                }
                IpBootState::Normal => {
                    let mut rxbuf = [0u8; EP_DATA_BUF_SIZE];

                    if let Ok(size) = ip.inner.read_packet(rxbuf.as_mut_slice()) {
                        // debug!("got packet: {:x},size {}", rxbuf[0..size], size);





                        match rxstate {
                            IpRxState::AwaitHeader => {
                                currheader = rxbuf[0..size].try_into().unwrap();
                                debug!("got message: {:?}", currheader);
                                currndp = rxbuf[(currheader.ndpidex as usize)..size]
                                    .try_into()
                                    .unwrap();
                                debug!("got ndp: {:?}", currndp);
                                if (currndp.datagrams[0].index as usize) < size {
                                    let diff = size - currndp.datagrams[0].index as usize;
                                    // the message starts in this packet, we can skip the locate state
                                    packetbuf[0..diff].copy_from_slice(&rxbuf[currndp.datagrams[0].index as usize..size]);
                                    bytes_copied += diff;
                                    // debug!("copied {} bytes",bytes_copied);
                                    rxstate = IpRxState::CollectData;
                                }else{
                                    currcnt = currndp.datagrams[0].index as usize - size; // start counting backwards until we reach the datagram
                                    rxstate = IpRxState::LocateDataStart;
                                }   
                            }
                            IpRxState::LocateDataStart => {
                                if currcnt <= size {
                                    // the start of the datagram is located on this packet, start collecting to buffer
                                    let diff = size - currcnt;
                                    packetbuf[0..diff].copy_from_slice(&rxbuf[currcnt..size]);
                                    bytes_copied += diff;
                                    rxstate = IpRxState::CollectData;
                                } else {
                                    currcnt -= size;
                                }
                            }
                            IpRxState::CollectData => {
                                let bytes_to_copy =(currndp.datagrams[0].length as usize - bytes_copied).min(size); 
                                packetbuf[bytes_copied..bytes_copied+bytes_to_copy].copy_from_slice(&rxbuf[0..bytes_to_copy]);
                                // debug!("copied {} bytes",bytes_copied);
                                bytes_copied += bytes_to_copy;
                                if currndp.datagrams[0].length as usize == bytes_copied {
                                    debug!(
                                        "finished copying message: {:x}",
                                        packetbuf[0..currndp.datagrams[0].length as usize]
                                    );
                                    bytes_copied = 0;
                                    rxstate = IpRxState::AwaitHeader;
                                }
                            }
                            IpRxState::Reply => {}
                        }
                    }
                }
            }
        }

        // let buf = [0u8;1024];
        // ip.inner.write_packet(&buf).unwrap();

        // tcpserv.eth_task();
    }
}

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
