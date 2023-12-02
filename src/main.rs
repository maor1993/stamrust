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
use cdc_ncm::{CDC_SUBCLASS_NCM, USB_CLASS_CDC};
mod intf;
mod ncm_netif;
use ncm_netif::{BufState, SyncBuf};
mod server;
use intf::UsbIp;
use server::TcpServer;

use crate::cdc_ncm::{EP_DATA_BUF_SIZE, NCM_MAX_OUT_SIZE};
use crate::intf::{NCMDatagram16, NCMDatagramPointerTable, NCMTransferHeader, ToBytes};


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

#[derive(PartialEq)]
enum IpBootState {
    Speed,
    Notify,
    Normal,
}

enum IpRxState {
    AwaitHeader,
    LocateDataStart,
    CollectData,
    Reply,
}

enum IpTxState {
    Copy,
    Write,
}


#[entry]
fn main() -> ! {
    init_heap();

    enable_crs();

    // disable Vddusb power isolation
    enable_usb_pwr();
    let mut periphs = ProjectPeriphs::new();

    let usb_bus = UsbBus::new(periphs.usb);

    let mut ip = UsbIp::new(&usb_bus);

    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x0483, 0xffff))
        .manufacturer("STMicroelectronics")
        .product("IP over USB Demonstrator")
        .serial_number("test")
        .device_release(0x0100)
        .device_class(USB_CLASS_CDC)
        .build();

    debug!("starting server...");
    let mut bootstate = IpBootState::Speed;
    let mut rxstate = IpRxState::AwaitHeader;
    let mut txstate = IpTxState::Copy;
    let mut currheader: NCMTransferHeader;
    let mut currndp = NCMDatagramPointerTable::default();
    let mut currcnt = 0;
    let mut bytes_copied = 0;
    let mut gotfirstpacket = false;
    let mut txtransactioncnt = 0;
    let mut tcpserv = TcpServer::init_server();
    let mut txheader = NCMTransferHeader::default();
    let mut txdatagram = NCMDatagramPointerTable::default();
    let mut usbtxbuf: [u8; 2048] = [0u8; 2048];
    let mut usbmsgtotlen = 0;
    loop {
        periphs.led.toggle();
        if usb_dev.poll(&mut [&mut ip.inner]) {
            match bootstate {
                IpBootState::Speed => {
                    if ip.send_speed_notificaiton().is_ok() {
                        bootstate = IpBootState::Notify
                    }
                }
                IpBootState::Notify => {
                    if ip.send_connection_notificaiton().is_ok() {
                        bootstate = IpBootState::Normal;
                        debug!("Sent notify!");
                    }
                }
                IpBootState::Normal => {
                    let mut usbbuf: [u8; EP_DATA_BUF_SIZE] = [0u8; EP_DATA_BUF_SIZE];
                    let (mut rxbuf, mut txbuf) = tcpserv.get_bufs();

                    if let IpTxState::Write = txstate {
                        // we can only send chunks of 64 bytes, so we will incrementally walk the buffer
                        let bytestocopy = (usbmsgtotlen - txtransactioncnt).min(EP_DATA_BUF_SIZE);
                        let msg = &usbtxbuf[txtransactioncnt..txtransactioncnt + bytestocopy];

                        if let Ok(size) = ip.inner.write_packet(msg) {
                            debug!("sent {} bytes", size);
                            //part of message sucesfully sent.
                            //increment the read pointer by size.
                            if txtransactioncnt >= usbmsgtotlen {
                                //we've finished sending this message.
                                //after sending, increment message sequence
                                txheader.sequence += 1;
                                txdatagram.datagrams.clear();
                                txstate = IpTxState::Copy;
                                txtransactioncnt = 0;
                            } else {
                                txtransactioncnt += size;
                            }
                        }
                    }

                    if rxbuf.busy == BufState::Await {
                        continue;
                    }

                    if let Ok(size) = ip.inner.read_packet(usbbuf.as_mut_slice()) {
                        // debug!("got packet: {:x},size {}", rxbuf[0..size], size);
                        match rxstate {
                            IpRxState::AwaitHeader => {
                                if size < core::mem::size_of::<NCMTransferHeader>() {
                                    continue;
                                }

                                currheader = match usbbuf[0..size].try_into().ok() {
                                    Some(x) => x,
                                    None => continue,
                                };
                                // debug!("got message: {:?}", currheader);
                                currndp = usbbuf[(currheader.ndpidex as usize)..size]
                                    .try_into()
                                    .unwrap();
                                // debug!("got ndp: {:?}", currndp);
                                rxbuf.busy = BufState::Writing;
                                if (currndp.datagrams[0].index as usize) < size {
                                    let diff = size - currndp.datagrams[0].index as usize;
                                    // the message starts in this packet, we can skip the locate state
                                    rxbuf.buf[0..diff].copy_from_slice(
                                        &usbbuf[currndp.datagrams[0].index as usize..size],
                                    );
                                    bytes_copied += diff;
                                    // debug!("copied {} bytes",bytes_copied);
                                    rxstate = IpRxState::CollectData;
                                } else {
                                    currcnt = currndp.datagrams[0].index as usize - size; // start counting backwards until we reach the datagram
                                    rxstate = IpRxState::LocateDataStart;
                                }
                            }
                            IpRxState::LocateDataStart => {
                                if currcnt <= size {
                                    // the start of the datagram is located on this packet, start collecting to buffer
                                    let diff = size - currcnt;
                                    rxbuf.buf[0..diff].copy_from_slice(&usbbuf[currcnt..size]);
                                    bytes_copied += diff;
                                    rxstate = IpRxState::CollectData;
                                } else {
                                    currcnt -= size;
                                }
                            }
                            IpRxState::CollectData => {
                                let bytes_to_copy =
                                    (currndp.datagrams[0].length as usize - bytes_copied).min(size);
                                rxbuf.buf[bytes_copied..bytes_copied + bytes_to_copy]
                                    .copy_from_slice(&usbbuf[0..bytes_to_copy]);
                                // debug!("copied {} bytes",bytes_copied);
                                bytes_copied += bytes_to_copy;
                                if currndp.datagrams[0].length as usize == bytes_copied {
                                    // debug!(
                                    //     "finished copying message: {:x}",
                                    //     packetbuf.buf[0..currndp.datagrams[0].length as usize]
                                    // );
                                    rxbuf.len = bytes_copied;
                                    bytes_copied = 0;
                                    rxstate = IpRxState::Reply;
                                    rxbuf.busy = BufState::Await;
                                    gotfirstpacket = true;
                                }
                            }
                            IpRxState::Reply => {
                                if let BufState::Writing = txbuf.busy {
                                    //we only need to copy the buffer
                                    if let IpTxState::Copy = txstate {
                                        //create a new datagram table entry for this message
                                        txdatagram.datagrams.push(NCMDatagram16 {
                                            index: 0x0020,
                                            length: txbuf.len as u16,
                                        });
                                        usbmsgtotlen = 0x0020 + txbuf.len;
                                        txheader.blocklen = usbmsgtotlen as u16;
                                        txdatagram.length = 0x0010;

                                        let headervec = txheader.conv_to_bytes();
                                        let datagramvec = txdatagram.conv_to_bytes();

                                        usbtxbuf[0x0000..0x000C]
                                            .copy_from_slice(headervec.as_slice());
                                        usbtxbuf[0x0010..0x001C]
                                            .copy_from_slice(datagramvec.as_slice());
                                        //TODO: this is only correct for 1 datagram
                                        usbtxbuf[0x0020..0x0020 + txbuf.len]
                                            .copy_from_slice(txbuf.buf[0..txbuf.len].as_ref());

                                        // info!("sending the following stream {:#02x}",usbtxbuf[0..usbmsgtotlen]);
                                        txstate = IpTxState::Write;
                                        txbuf.busy = BufState::Empty;
                                        rxstate = IpRxState::AwaitHeader;
                                    }
                                } else {
                                    //missed this round.
                                    rxstate = IpRxState::AwaitHeader;
                                }
                            }
                        }
                    }
                }
            }
        } else if bootstate == IpBootState::Normal && gotfirstpacket {
            tcpserv.eth_task();
        }
    }
}

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
