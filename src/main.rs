#![no_std]
#![no_main]

//runtime
use cortex_m_rt::entry;
use defmt::{debug, info};
use defmt_rtt as _;
use embedded_alloc::Heap;
use panic_probe as _;
// hal
use stm32l4xx_hal::gpio::{Pin,Output,PushPull};
use stm32l4xx_hal::usb::{Peripheral, UsbBus};
use stm32l4xx_hal::{prelude::*, stm32};
use usb_device::prelude::*;

//app
mod cdc_ncm;
use crate::cdc_ncm::USB_CLASS_CDC;
mod intf;
use crate::intf::UsbIp;

mod server;
use server::TcpServer;

mod ncm_netif;

mod usbipserver;
use usbipserver::UsbIpManager;



type LedPin = Pin<Output<PushPull>,stm32l4xx_hal::gpio::H8,'A',9>;

struct ProjectPeriphs{
    led : LedPin,
    usb : Peripheral,
}

impl ProjectPeriphs{
    fn new() -> Self{
        
        let dp = stm32::Peripherals::take().unwrap();

        let mut flash = dp.FLASH.constrain();
        let mut rcc = dp.RCC.constrain();
        let mut pwr = dp.PWR.constrain(&mut rcc.apb1r1);
    
        let _clocks = rcc
            .cfgr
            .hsi48(true)
            .sysclk(80.MHz())
            .freeze(&mut flash.acr, &mut pwr);
    


        let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);
        let mut led = gpioa
            .pa9
            .into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper);
        led.set_low(); // Turn off
    
        let usb = Peripheral {
            usb:dp.USB,
            pin_dm: gpioa
                .pa11
                .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
            pin_dp: gpioa
                .pa12
                .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
        };
    
        ProjectPeriphs { led, usb }

    }
}




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




#[entry]
fn main() -> ! {
    init_heap();

    enable_crs();

    // disable Vddusb power isolation
    enable_usb_pwr();
    let mut periphs = ProjectPeriphs::new();

    let usb_bus = UsbBus::new(periphs.usb);

    let ip = UsbIp::new(&usb_bus);

    let usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x0483, 0xffff))
        .manufacturer("STMicroelectronics")
        .product("IP over USB Demonstrator")
        .serial_number("test")
        .device_release(0x0100)
        .device_class(USB_CLASS_CDC)
        .build();

    info!("starting server...");
    let mut tcpserv = TcpServer::init_server();
    let mut usbip = UsbIpManager::new(ip,usb_dev);
    loop {
        periphs.led.toggle();
        usbip.run_loop(tcpserv.get_bufs());
        tcpserv.eth_task();
    }
}

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
