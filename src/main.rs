#![no_std]
#![no_main]
use core::cell::RefCell;

//runtime
use cortex_m_rt::entry;
use cortex_m_rt::exception;
use critical_section::{with, Mutex};
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
    timer::{OutputCompare, TimChannel, Timer, TimerConfig},
    usb::{self, Peripheral, UsbBus},
};

//app
mod cdc_ncm;
mod ncm_api;
use ncm_api::NcmApiManager;

mod server;
use server::TcpServer;

mod ncm_netif;

mod usbipserver;
use usb_device::class_prelude::UsbBusAllocator;
use usbipserver::UsbIpManager;

static TICKS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0u32));

defmt::timestamp!("{=u32}",{
   get_counter() 
});

fn increase_counter() {
    with(|cs| {
        *TICKS.borrow(cs).borrow_mut() += 1;
    })
}
fn get_counter() -> u32 {
    with(|cs| *TICKS.borrow(cs).borrow())
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
    fn set_duty(&mut self, led: RgbLed, duty: u16) {
        let max_duty = self.rgb.get_max_duty();

        let channel = match led {
            RgbLed::Red => TimChannel::C1,
            RgbLed::Green => TimChannel::C2,
            RgbLed::Blue => TimChannel::C3,
        };

        self.rgb.set_duty(channel, max_duty * duty / 100);
    }

    fn active_all_pwms(&mut self) {
        self.rgb
            .enable_pwm_output(TimChannel::C1, OutputCompare::Pwm1, 0.0);
        self.rgb
            .enable_pwm_output(TimChannel::C2, OutputCompare::Pwm1, 0.0);
        self.rgb
            .enable_pwm_output(TimChannel::C3, OutputCompare::Pwm1, 0.0);
    }
}

make_globals!((USBCON, UsbIpManager<'_, UsbBus<Peripheral>>));

static mut USB_BUS: Option<UsbBusAllocator<UsbBus<Peripheral>>> = None;

struct ProjectPeriphs {
    arm: cortex_m::Peripherals,
    // sanity_led : LedPin,
    // rgb: RgbControl,
    led: Pin,
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

        let _usb_dm = Pin::new(Port::A, 11, PinMode::Alt(14));
        let _usb_dp = Pin::new(Port::A, 12, PinMode::Alt(14));

        arm.SYST.set_reload((clock_cfg.systick() / 8_000) - 1);
        arm.SYST.enable_counter();
        arm.SYST.enable_interrupt();

        // let pwm_timer = Timer::new_tim1(
        //     dp.TIM1,
        //     2_400.,
        //     TimerConfig {
        //         auto_reload_preload: true,
        //         // Setting auto reload preload allow changing frequency (period) while the timer is running.
        //         ..Default::default()
        //     },
        //     &clock_cfg,
        // );

        // let rgb = RgbControl::new(pwm_timer);
        let usb = Peripheral { regs: dp.USB };
        let _rng = Rng::new(dp.RNG);
        let mut led = Pin::new(Port::A, 8, PinMode::Output);
        led.output_speed(stm32_hal2::gpio::OutputSpeed::Low);
        // ProjectPeriphs {arm, usb, rgb }
        ProjectPeriphs { arm, usb, led }
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
    // periphs.rgb.active_all_pwms();
    // periphs.rgb.set_duty(RgbLed::Red, 20);
    // periphs.rgb.set_duty(RgbLed::Green, 0);
    // periphs.rgb.set_duty(RgbLed::Blue, 0);

    let mut perfcounter = 0;
    let mut lastlooptime = 0;

    //move the global variables to the critical seciton
    with(|cs| unsafe {
        let usbipmanager = UsbIpManager::new(USB_BUS.as_ref().unwrap());
        USBCON.borrow(cs).replace(Some(usbipmanager));
    });
    unsafe {
        NVIC::unmask(interrupt::USB_FS);
        periphs.arm.NVIC.set_priority(interrupt::USB_FS, 1);
    }
    loop {
        let looptime = get_counter();
        with(|cs| {
            access_global!(USBCON, usbcon, cs);
            ncmapi.process_messages(tcpserv.get_bufs(), usbcon.get_bufs());
        });

        tcpserv.eth_task(looptime);
        lastlooptime =
            finalize_perfcounter(&mut perfcounter, looptime, lastlooptime, &mut periphs.led);
        perfcounter += 1;
    }
}

fn finalize_perfcounter(cnt: &mut u32, looptime: u32, lastlooptime: u32, led: &mut Pin) -> u32 {
    if looptime.saturating_sub(lastlooptime) >= 1000 {
        // info!("seconds:{} loops: {}", looptime / 1000, cnt);
        *cnt = 0;
        led.toggle();
        looptime
    } else {
        lastlooptime
    }
}

#[interrupt]
/// Interrupt handler for USB (serial)
fn USB_FS() {
    with(|cs| {
        access_global!(USBCON, usbipmanager, cs);
        usbipmanager.run_loop();
    })
}

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
