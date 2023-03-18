#![no_std]
#![no_main]


use panic_rtt_target as _; // you can put a breakpoint on `rust_begin_unwind` to catch panics

use cortex_m::prelude::*;

use stm32l4xx_hal::{prelude::*, stm32,pac,hal::prelude::*};
use stm32l4xx_hal::delay::Delay;
use cortex_m_rt::entry;
use rtt_target::{rprintln, rtt_init_print};



#[entry]
fn main() -> ! {
    rtt_init_print!();
    if let (Some(dp), Some(cp)) = (
        pac::Peripherals::take(),
        cortex_m::peripheral::Peripherals::take(),
    ) {

        let mut flash = dp.FLASH.constrain(); // .constrain();
        let mut rcc = dp.RCC.constrain();
        let mut pwr = dp.PWR.constrain(&mut rcc.apb1r1);
        let clocks = rcc.cfgr.sysclk(80.MHz()).freeze(&mut flash.acr, &mut pwr);


        
        let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);
        
        let mut led = gpioa.pa9.into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper);

        let mut delay = Delay::new(cp.SYST,clocks);
        

        loop {
            rprintln!("hello!");
            led.toggle();
            delay.delay_ms(100_u32);
        }
    }
    loop {}
}
