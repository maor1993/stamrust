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
    let mut state = IpBootState::Speed;
    // let mut tcpserv = TcpServer::init_server(ip.ip_in.borrow_mut(),ip.ip_out.borrow_mut());
    loop {
        if usb_dev.poll(&mut [&mut ip.inner]) {
            match state {
                IpBootState::Speed => {
                    if ip.send_speed_notificaiton().is_ok() {
                        state = IpBootState::Notify
                    }
                }
                IpBootState::Notify => {
                    if ip.send_connection_notificaiton().is_ok() {
                        state = IpBootState::Normal;
                        debug!("Sent notify!");
                    }
                }
                IpBootState::Normal => {
                    let mut buf = [0u8; 1024];
                    if ip.inner.read_packet(buf.as_mut_slice()).is_ok() {
                        debug!("got packet: {:?}", buf);
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
