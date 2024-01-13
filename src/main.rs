#![no_std]
#![no_main]

use core::cell::RefCell;

//runtime
use concurrent_queue::ConcurrentQueue;
// use cortex_m::interrupt::{CriticalSection, Mutex};
// use cortex_m::peripheral::SYST;
use cortex_m_rt::entry;
use cortex_m_rt::exception;
use defmt::info;
use defmt_rtt as _;
use embedded_alloc::Heap;
use panic_probe as _;
use usbipserver::Usbtransaciton;
// hal
use stm32l4xx_hal::delay::Delay;
use stm32l4xx_hal::device::TIM1;
// use stm32l4xx_hal::gpio::{Output, Pin, PushPull};
// use stm32l4xx_hal::interrupt;
use stm32l4xx_hal::pwm::*;
use stm32l4xx_hal::rng::Rng;
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

static mut TICKS: RefCell<u32> = RefCell::new(0u32);

fn increase_counter() {
    unsafe { *TICKS.borrow_mut() += 1 };
}
fn get_counter() -> u32 {
    unsafe { *TICKS.borrow() }
}

#[exception]
fn SysTick() {
    increase_counter();
}

// type LedPin = Pin<Output<PushPull>, stm32l4xx_hal::gpio::H8, 'A', 8>;
type Rgb = (Pwm<TIM1, C1>, Pwm<TIM1, C2>, Pwm<TIM1, C3>);

enum RgbLed {
    Red,
    Green,
    Blue,
}

struct RgbControl {
    rgb: Rgb,
}
impl RgbControl {
    fn new(rgb: Rgb) -> Self {
        RgbControl { rgb }
    }
    fn set_duty(&mut self, led: RgbLed, duty: u16) {
        let duty_actual = self.rgb.0.get_max_duty() * (100 - duty) / 100;
        match led {
            RgbLed::Red => self.rgb.0.set_duty(duty_actual),
            RgbLed::Green => self.rgb.1.set_duty(duty_actual),
            RgbLed::Blue => self.rgb.2.set_duty(duty_actual),
        }
    }

    fn active_all_pwms(&mut self) {
        self.rgb.0.enable();
        self.rgb.1.enable();
        self.rgb.2.enable();
    }
}

struct ProjectPeriphs {
    // sanity_led : LedPin,
    rgb: RgbControl,
    usb: Peripheral,
    rng: Rng,
    delay: Delay,
}
impl ProjectPeriphs {
    fn new() -> Self {
        let dp = stm32::Peripherals::take().unwrap();
        let mut arm = cortex_m::Peripherals::take().unwrap();

        let mut flash = dp.FLASH.constrain();
        let mut rcc = dp.RCC.constrain();
        let mut pwr = dp.PWR.constrain(&mut rcc.apb1r1);

        let _clocks = rcc
            .cfgr
            .hsi48(true)
            .sysclk(80.MHz())
            .freeze(&mut flash.acr, &mut pwr);

        let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);
        arm.SYST.set_reload(_clocks.sysclk().to_kHz() - 1);
        arm.SYST.enable_counter();
        arm.SYST.enable_interrupt();
        let c1 = gpioa
            .pa8
            .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh);
        let c2 = gpioa
            .pa9
            .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh);
        let c3 = gpioa
            .pa10
            .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh);

        let rgb = dp.TIM1.pwm((c1, c2, c3), 1.MHz(), _clocks, &mut rcc.apb2);
        let rgbcon = RgbControl::new(rgb);
        let usb = Peripheral {
            usb: dp.USB,
            pin_dm: gpioa
                .pa11
                .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
            pin_dp: gpioa
                .pa12
                .into_alternate(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrh),
        };
        let rng = dp.RNG.enable(&mut rcc.ahb2, _clocks);

        ProjectPeriphs {
            // sanity_led: gpioa.pa8.into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper),
            delay: Delay::new(arm.SYST, _clocks),
            usb,
            rgb: rgbcon,
            rng,
        }
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
    let mut tcpserv = TcpServer::init_server(periphs.rng.get_random_data());
    let mut usbip = UsbIpManager::new(ip, usb_dev);
    periphs.rgb.active_all_pwms();
    periphs.rgb.set_duty(RgbLed::Red, 20);
    periphs.rgb.set_duty(RgbLed::Green, 0);
    periphs.rgb.set_duty(RgbLed::Blue, 0);

    let mut perfcounter = 0;
    let mut lastlooptime = 0;

    let mut rbusbncm = ConcurrentQueue::<Usbtransaciton>::bounded(4);
    let mut rbncmusb = ConcurrentQueue::<Usbtransaciton>::bounded(4);

    loop {
        let looptime = get_counter();
        usbip.run_loop(tcpserv.get_bufs(), (&mut rbusbncm, &mut rbncmusb));
        tcpserv.eth_task(looptime);
        lastlooptime = finalize_perfcounter(&mut perfcounter, looptime, lastlooptime);
        perfcounter += 1;
    }
}

fn finalize_perfcounter(cnt: &mut u32, looptime: u32, lastlooptime: u32) -> u32 {
    if looptime.saturating_sub(lastlooptime) >= 1000 {
        // info!("looptime:{} loops: {}", looptime, cnt);
        *cnt = 0;
        looptime
    } else {
        lastlooptime
    }
}

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
