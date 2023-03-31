extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};

use crate::ncm_netif::UsbIpPhy;
use defmt::println;

pub fn init_server() {
    // Create interface
    let mut device = UsbIpPhy::new();
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

    loop {
        let timestamp = Instant::from_millis_const(0); //FIXME: replace with timestamp generator
        iface.poll(timestamp, &mut device, &mut sockets);

        // tcp:6969: respond "hello"
        let socket = sockets.get_mut::<tcp::Socket>(tcp1_handle);
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

        // phy_wait(fd, iface.poll_delay(timestamp, &sockets)).expect("wait error");
    }
}
