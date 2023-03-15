#![no_std]
#![no_main]


use panic_halt as _; // you can put a breakpoint on `rust_begin_unwind` to catch panics

use cortex_m::{prelude::*, delay::Delay};

use stm32_hal2 as hal;
use hal::gpio::{Pin,PinMode, Port};
use hal::{
    pac,
    clocks::Clocks

};
use cortex_m_rt::entry;
use rtt_target::{rprintln, rtt_init_print};


#[entry]
fn main() -> ! {
    rtt_init_print!();
    if let (Some(dp), Some(cp)) = (
        pac::Peripherals::take(),
        cortex_m::peripheral::Peripherals::take(),
    ) {

        let clockcfg  = Clocks::default();
        clockcfg.setup().unwrap();

        

        let mut led = Pin::new(Port::A,9,PinMode::Output);
        let mut delay = Delay::new(cp.SYST,clockcfg.systick());
        

        loop {
            rprintln!("hello!");
            led.toggle();
            delay.delay_ms(100_u32);
        }
    }
    loop {}
}
