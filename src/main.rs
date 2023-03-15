#![no_std]
#![no_main]

// Not all stm32 devices have a Clock Recovery System (CRS) such as the stm32l476.
// For these device families, we can't use the CRS or clocks that depend on the
// CRS such as the HSI. See
// https://github.com/David-OConnor/stm32-hal/issues/64 for further details.
//
// The following example was run and tested on the STM32L476ZGTx varient.

use panic_rtt_target as _;
use rtt_target::{rtt_init_print,rprintln};

pub mod usb_pow {
    use stm32_hal2::pac::{PWR, RCC};

    /// Enables VddUSB power supply
    pub fn enable_usb_pwr() {
        // Enable PWR peripheral
        let rcc = unsafe { &(*RCC::ptr()) };
        rcc.apb1enr1.modify(|_, w| w.pwren().set_bit());

        // Enable VddUSB
        let pwr = unsafe { &*PWR::ptr() };
        pwr.cr2.modify(|_, w| w.usv().set_bit());
    }
}

#[rtic::app(device = pac, dispatchers = [USART1])]
mod app {

    use cortex_m::asm;

    use stm32_hal2::{
        self,
        clocks::*,
        gpio::{Pin, PinMode, Port},
        pac,
        usb::Peripheral
    };

    use stm32_hal2::clocks::Clk48Src;
    use stm32_hal2::usb::UsbBus;

    use usb_device::prelude::*;

    use usbd_serial::SerialPort;
    use usbd_serial::USB_CLASS_CDC;

    use usb_device::class_prelude::UsbBusAllocator;

    use super::*;

    pub struct PeripheralUsb {
        pub serial: SerialPort<'static, UsbBus<Peripheral>>,
        pub device: UsbDevice<'static, UsbBus<Peripheral>>,
    }

    #[shared]
    struct Shared {
        peripheral_usb: PeripheralUsb,
    }

    #[local]
    struct Local {}

    #[init(local = [
           usb_bus: Option<UsbBusAllocator<UsbBus<Peripheral>>> = None,
    ])]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        rtt_init_print!();
        // Use the msi clock for the 48 Mhz input to the USB peripheral rather
        // than the hsi.
            let clock_cfg = Clocks {
            input_src: InputSrc::Pll(PllSrc::Msi(MsiRange::R4M)),
            pll:PllCfg{
                divm: Pllm::Div1,
                divn: 40,
                divr: Pllr::Div2,
                divq: Pllr::Div2,
                ..Default::default()
            },
            clk48_src: Clk48Src::Hsi48,
            ..Default::default()
        };
        usb_pow::enable_usb_pwr();

        clock_cfg.setup().unwrap();
       
        let _usb_dm = Pin::new(Port::A, 11, PinMode::Alt(10));
        let _usb_dp = Pin::new(Port::A, 12, PinMode::Alt(10));

        let usb1 = Peripheral { regs: cx.device.USB };
        let bus = cx.local.usb_bus.insert(UsbBus::new(usb1));
        let serial = SerialPort::new(bus);
        let device = UsbDeviceBuilder::new(bus, UsbVidPid(0x0483, 0xffff))
            .manufacturer("Fake Company")
            .product("Serial Port")
            .serial_number("SN")
            .device_class(USB_CLASS_CDC)
            .build();
        let peripheral_usb = PeripheralUsb { serial, device };

        rprintln!("finished setup.");
        (Shared { peripheral_usb }, Local {}, init::Monotonics())
    }

    #[idle()]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            asm::nop()
        }
    }

    #[task(binds = USB_FS, shared = [peripheral_usb])]
    fn usb_say_hello(cx: usb_say_hello::Context) {
        rprintln!("usb_say_hello");
        let mut peripheral_usb = cx.shared.peripheral_usb;

        peripheral_usb.lock(|PeripheralUsb { device, serial }| loop {
            if !device.poll(&mut [serial]) {
                continue;
            }

            // Something in the usb buffer. Process it.
            let mut buf = [0u8; 64];

            match serial.read(&mut buf[..]) {
                Ok(count) => {
                    // Echo back to the serial console.
                    serial.write(&buf[..count]).unwrap();
                }
                Err(UsbError::WouldBlock) => {
                    // Add error handling
                }
                Err(_err) => {
                    // Add error handling
                }
            }
        })
    }
}