#![no_std]
#![no_main]
use core::cell::RefCell;

//runtime
use cortex_m_rt::entry;
use cortex_m_rt::exception;
use critical_section::{with, Mutex};
use defmt::debug;
use defmt::info;
use defmt_rtt as _;
use embedded_alloc::Heap;
use panic_probe as _;
use stm32_hal2::access_global;
use stm32_hal2::make_globals;
use stm32_hal2::pac::TIM1;
// hal
use cortex_m::peripheral::NVIC;
use stm32_hal2::{
    clocks::{self, Clk48Src, Clocks, CrsSyncSrc},
    gpio::{Pin, PinMode, Port},
    pac::{self, interrupt},
    rng::{self, Rng},
    timer::{OutputCompare, TimChannel, Timer},
    usb::{self, Peripheral, UsbBus},
};

//app
mod cdc_ncm;
mod ncm_api;
use ncm_api::NcmApiManager;

mod http;
mod server;
use server::TcpServer;

mod ncm_netif;

mod usbipserver;
use usb_device::class_prelude::UsbBusAllocator;
use usbipserver::UsbIpManager;

static TICKS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0u32));
static LPS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0u32));
static RGB: Mutex<RefCell<(u8, u8, u8)>> = Mutex::new(RefCell::new((0, 0, 0)));

defmt::timestamp!("{=u32}", { get_counter() });

fn increase_counter() {
    with(|cs| {
        *TICKS.borrow(cs).borrow_mut() += 1;
    })
}
fn get_counter() -> u32 {
    with(|cs| *TICKS.borrow(cs).borrow())
}
pub fn set_rgb(val: (u8, u8, u8)) {
    with(|cs| {
        *RGB.borrow(cs).borrow_mut() = val;
    })
}
pub fn get_rgb() -> (u8, u8, u8) {
    with(|cs| *RGB.borrow(cs).borrow())
}

fn set_lps(val: u32) {
    with(|cs| {
        *LPS.borrow(cs).borrow_mut() = val;
    })
}
pub fn get_lps() -> u32 {
    with(|cs| *LPS.borrow(cs).borrow())
}

#[exception]
fn SysTick() {
    increase_counter();
}

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
    fn set_duty(&mut self, led: RgbLed, duty: u8) {
        let max_duty = self.rgb.get_max_duty();

        let channel = match led {
            RgbLed::Red => TimChannel::C1,
            RgbLed::Green => TimChannel::C2,
            RgbLed::Blue => TimChannel::C3,
        };
        self.rgb
            .set_duty(channel, (max_duty / u8::MAX as u16) * duty as u16);
    }
    fn tim1_errata(&mut self) {
        let tim1 = unsafe { &(*TIM1::ptr()) };
        tim1.bdtr.write(|w| w.moe().set_bit());
        tim1.egr.write(|w| w.ug().set_bit());
    }

    fn active_all_pwms(&mut self) {
        self.rgb
            .enable_pwm_output(TimChannel::C1, OutputCompare::Pwm1, 1.0);
        self.rgb
            .enable_pwm_output(TimChannel::C2, OutputCompare::Pwm1, 1.0);
        self.rgb
            .enable_pwm_output(TimChannel::C3, OutputCompare::Pwm1, 1.0);
        self.rgb.enable();
        self.tim1_errata();
    }
}

make_globals!((USBCON, UsbIpManager<'_, UsbBus<Peripheral>>));

static mut USB_BUS: Option<UsbBusAllocator<UsbBus<Peripheral>>> = None;

struct ProjectPeriphs {
    arm: cortex_m::Peripherals,
    rgb: RgbControl,
    // led: Pin,
    usb: Peripheral,
    clk_cfg: Clocks,
}
impl ProjectPeriphs {
    fn new() -> Self {
        let dp = pac::Peripherals::take().unwrap();
        let mut arm = cortex_m::Peripherals::take().unwrap();

        let clk_cfg = Clocks {
            // Enable the HSI48 oscillator, so we don't need an external oscillator, and
            // aren't restricted in our PLL config.
            hsi48_on: true,
            clk48_src: Clk48Src::Hsi48,
            ..Default::default()
        };
        clk_cfg.setup().unwrap();
        clocks::enable_crs(CrsSyncSrc::Usb);

        dp.RCC.apb1enr1.modify(|_, w| w.pwren().set_bit());
        usb::enable_usb_pwr();

        let _usb_dm = Pin::new(Port::A, 11, PinMode::Alt(14));
        let _usb_dp = Pin::new(Port::A, 12, PinMode::Alt(14));

        let _rgb_r = Pin::new(Port::A, 8, PinMode::Alt(1));
        let _rgb_g = Pin::new(Port::A, 9, PinMode::Alt(1));
        let _rgb_b = Pin::new(Port::A, 10, PinMode::Alt(1));

        arm.SYST.set_reload((clk_cfg.systick() / 8_000) - 1);
        arm.SYST.enable_counter();
        arm.SYST.enable_interrupt();

        let pwm_timer = Timer::new_tim1(
            dp.TIM1,
            10000.,
            stm32_hal2::timer::TimerConfig {
                auto_reload_preload: true,
                // Setting auto reload preload allow changing frequency (period) while the timer is running.
                ..Default::default()
            },
            &clk_cfg,
        );

        let rgb = RgbControl::new(pwm_timer);
        let usb = Peripheral { regs: dp.USB };
        let _rng = Rng::new(dp.RNG);

        // ProjectPeriphs {arm, usb, rgb }
        ProjectPeriphs {
            arm,
            usb,
            rgb,
            clk_cfg,
        }
    }
    fn enable_irqs(&mut self) {
        unsafe {
            NVIC::unmask(interrupt::USB_FS);
            self.arm.NVIC.set_priority(interrupt::USB_FS, 1);
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
    unsafe {
        USB_BUS = Some(usb_bus);
    }
    let mut ncmapi = NcmApiManager::new();

    info!("starting server...");
    let mut tcpserv = TcpServer::init_server(rng::read() as u32);
    periphs.rgb.active_all_pwms();

    let mut perfcounter = 0;
    let mut lastlooptime = 0;

    //move the global variables to the critical seciton
    with(|cs| unsafe {
        let usbipmanager = UsbIpManager::new(USB_BUS.as_ref().unwrap());
        USBCON.borrow(cs).replace(Some(usbipmanager));
    });

    loop {
        let looptime = get_counter();
        with(|cs| {
            access_global!(USBCON, usbcon, cs);
            usbcon.run_loop();
            ncmapi.process_messages(tcpserv.get_bufs(), usbcon.get_bufs());
        });
        tcpserv.eth_task(looptime);
        handle_incoming_rgb_requests(&mut periphs.rgb);
        lastlooptime = finalize_perfcounter(&mut perfcounter, looptime, lastlooptime);
        perfcounter += 1;
    }
}

fn handle_incoming_rgb_requests(rgb: &mut RgbControl) {
    let (r, g, b) = get_rgb();
    rgb.set_duty(RgbLed::Red, 255 - r);
    rgb.set_duty(RgbLed::Green, 255 - g);
    rgb.set_duty(RgbLed::Blue, 255 - b);
}

fn finalize_perfcounter(cnt: &mut u32, looptime: u32, lastlooptime: u32) -> u32 {
    if looptime.saturating_sub(lastlooptime) >= 1000 {
        debug!("seconds:{} loops: {}", looptime / 1000, cnt);
        set_lps(*cnt);
        *cnt = 0;
        // led.toggle();

        looptime
    } else {
        lastlooptime
    }
}

// #[interrupt]
// /// Interrupt handler for USB (serial)
// fn USB_FS() {
//     with(|cs| {
//         access_global!(USBCON, usbipmanager, cs);
//         usbipmanager.run_loop();
//     })
// }

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
