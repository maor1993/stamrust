extern crate alloc;

use core::cell::RefMut;

use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};
use usb_device::class_prelude::UsbBus;
use usb_device::prelude::UsbDevice;

use crate::intf::{UsbIp, UsbIpIn, UsbIpOut};
use crate::ncm_netif::UsbIpPhy;
use defmt::println;

pub struct TcpServer<'a> {
    device: UsbIpPhy<'a>,
    iface: Interface,
    sockets: SocketSet<'a>,
    tcp1_handle: SocketHandle,
}

impl<'a> TcpServer<'a> {
    pub fn init_server(tx:RefMut<'a,UsbIpIn>,rx:RefMut<'a,UsbIpOut>) -> Self
    {
        // Create interface
        let mut device = UsbIpPhy::new(tx, rx);
        let mut config = Config::new();
        config.random_seed = 0; //FIXME: get a random seed from hardware

        let mut iface = Interface::new(config, &mut device);
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(IpCidr::new(IpAddress::v4(192, 168, 69, 1), 24))
                .unwrap();
        });
        iface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(192, 168, 69, 100))
            .unwrap();

        let tx_buf: Vec<u8> = vec![0; 128];
        let rx_buf: Vec<u8> = vec![0; 64];

        // // Create sockets
        let tcp1_rx_buffer = tcp::SocketBuffer::new(rx_buf);
        let tcp1_tx_buffer = tcp::SocketBuffer::new(tx_buf);
        let tcp1_socket = tcp::Socket::new(tcp1_rx_buffer, tcp1_tx_buffer);

        let mut sockets = SocketSet::new(vec![]);
        let tcp1_handle = sockets.add(tcp1_socket);

        TcpServer {
            device,
            iface,
            sockets,
            tcp1_handle,
        }
    }
    pub fn eth_task(&mut self) {
        let timestamp = Instant::from_millis_const(0); //FIXME: replace with timestamp gene
        self.iface
            .poll(timestamp, &mut self.device, &mut self.sockets);
        // tcp:6969: respond "hello"
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp1_handle);
        if !socket.is_open() {
            socket.listen(6969).unwrap();
        }
        if socket.can_send() {
            println!("tcp:6969 send greeting");
            socket
                .send_slice(b"my name is jeff")
                .expect("failed to send message");
            println!("tcp:6969 close");
            socket.close();
        }
    }
}

// pub fn eth_task(){

// }

//     loop {

//         let timestamp = Instant::from_millis_const(0); //FIXME: replace with timestamp generator

//         iface.poll(timestamp, &mut device, &mut sockets);

//         // tcp:6969: respond "hello"
//         let socket = sockets.get_mut::<tcp::Socket>(tcp1_handle);
//         if !socket.is_open() {
//             socket.listen(6969).unwrap();
//         }

//         if socket.can_send() {
//             println!("tcp:6969 send greeting");
//             socket
//                 .send_slice(b"my name is jeff")
//                 .expect("failed to send message");
//             println!("tcp:6969 close");
//             socket.close();
//         }

//         // phy_wait(fd, iface.poll_delay(timestamp, &sockets)).expect("wait error");
//     }
// }
