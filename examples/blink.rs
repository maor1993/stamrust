#![no_std]
#![no_main]

use defmt::*;

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}

use cortex_m::prelude::*;

use cortex_m_rt::entry;
use stm32l4xx_hal::delay::Delay;
use stm32l4xx_hal::{hal::prelude::*, pac, prelude::*, stm32};

#[entry]
fn main() -> ! {
    if let (Some(dp), Some(cp)) = (
        pac::Peripherals::take(),
        cortex_m::peripheral::Peripherals::take(),
    ) {
        let mut flash = dp.FLASH.constrain(); // .constrain();
        let mut rcc = dp.RCC.constrain();
        let mut pwr = dp.PWR.constrain(&mut rcc.apb1r1);
        let clocks = rcc.cfgr.sysclk(80.MHz()).freeze(&mut flash.acr, &mut pwr);

        let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);

        let mut led = gpioa
            .pa9
            .into_push_pull_output(&mut gpioa.moder, &mut gpioa.otyper);

        let mut delay = Delay::new(cp.SYST, clocks);

        loop {
            println!("hello!");
            led.toggle();
            delay.delay_ms(100_u32);
        }
    }
    loop {}
}
