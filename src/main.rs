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
use stm32_hal2::pac::TIM1;
use usbipserver::Usbtransaciton;
// hal
use stm32_hal2::{
    clocks::{self, Clk48Src, Clocks, CrsSyncSrc},
    gpio::{Pin, PinMode, Port},
    pac,
    usb::{self,Peripheral, UsbBus},
    rng::{self,Rng},
    timer::{Timer,TimChannel,OutputCompare,TimerConfig}
    
};


use usb_device::prelude::*;

//app
mod cdc_ncm;
use crate::cdc_ncm::{USB_CLASS_CDC,CDC_SUBCLASS_NCM};
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

enum RgbLed {
    Red,
    Green,
    Blue,
}

struct RgbControl {
    rgb: Timer<TIM1>,
}
impl RgbControl {
    fn new(rgb: Timer<TIM1>) -> Self {
        RgbControl { rgb }
    }
    fn set_duty(&mut self, led: RgbLed, duty: u16) {
        let max_duty = self.rgb.get_max_duty();

        let channel = match led {
            RgbLed::Red => TimChannel::C1,
            RgbLed::Green => TimChannel::C2,
            RgbLed::Blue => TimChannel::C3,
        };

        self.rgb.set_duty(channel, max_duty*duty/100);
    }

    fn active_all_pwms(&mut self) {
        self.rgb.enable_pwm_output(TimChannel::C1, OutputCompare::Pwm1, 0.0);
        self.rgb.enable_pwm_output(TimChannel::C2, OutputCompare::Pwm1, 0.0);
        self.rgb.enable_pwm_output(TimChannel::C3, OutputCompare::Pwm1, 0.0);
    }
}

struct ProjectPeriphs {
    // sanity_led : LedPin,
    rgb: RgbControl,
    usb: Peripheral,
}
impl ProjectPeriphs {
    fn new() -> Self {
        let dp = pac::Peripherals::take().unwrap();
        let mut arm = cortex_m::Peripherals::take().unwrap();

        let clock_cfg = Clocks {
            // Enable the HSI48 oscillator, so we don't need an external oscillator, and
            // aren't restricted in our PLL config.
            hsi48_on: true,
            clk48_src: Clk48Src::Hsi48,
            ..Default::default()
        };
        clock_cfg.setup().unwrap();
        clocks::enable_crs(CrsSyncSrc::Usb);


        dp.RCC.apb1enr1.modify(|_, w| w.pwren().set_bit());
        usb::enable_usb_pwr();

        let _usb_dm = Pin::new(Port::A,11,PinMode::Alt(14));
        let _usb_dp = Pin::new(Port::A,12,PinMode::Alt(14));

        arm.SYST.set_reload(clock_cfg.sysclk()/1000 - 1);
        arm.SYST.enable_counter();
        arm.SYST.enable_interrupt();


        let pwm_timer = Timer::new_tim1(
            dp.TIM1,
            2_400.,
            TimerConfig {
                auto_reload_preload: true,
                // Setting auto reload preload allow changing frequency (period) while the timer is running.
                ..Default::default()
            },
            &clock_cfg,
        ); 


        let rgbcon = RgbControl::new(pwm_timer);
        let usb = Peripheral {
            regs: dp.USB
        };
        let _rng = Rng::new(dp.RNG);

        ProjectPeriphs {
            // sanity_led: gpioa.pa8.into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper),
            // delay: Delay::new(arm.SYST, _clocks),
            usb,
            rgb: rgbcon,
            // rng,
        }
    }
}

#[global_allocator]
static HEAP: Heap = Heap::empty();


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

    let mut periphs = ProjectPeriphs::new();
    let usb_bus = UsbBus::new(periphs.usb);
    let ip = UsbIp::new(&usb_bus);
    let usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x0483, 0xffff))
        .device_class(USB_CLASS_CDC)
        .device_sub_class(CDC_SUBCLASS_NCM)
        .build();

    info!("starting server...");
    let mut tcpserv = TcpServer::init_server(rng::read() as u32);
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
        info!("seconds:{} loops: {}", looptime/1000, cnt);
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
